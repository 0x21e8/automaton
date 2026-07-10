#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

TASK_FILE="${REPO_ROOT}/docs/specs/2026-03-01-single-evm-steward-tasklist.md"
MAX_STEPS=200
AUTO_APPROVE=0
DRY_RUN=0
MODEL=""
INCLUDE_GUARDRAILS=0
LOG_DIR="${REPO_ROOT}/.local/ralph-loop"
RUNNER="auto"

usage() {
  cat <<'USAGE'
Usage: scripts/ralph-loop.sh [options]

Runs checklist tasks one-by-one using fresh Codex exec sessions.

Options:
  --task-file <path>          Tasklist markdown file (default: steward tasklist)
  --max-steps <n>             Maximum loop steps (default: 200)
  --model <name>              Optional model override for codex exec
  --runner <auto|codex>       CLI runner selection (default: auto)
  --auto                       Do not ask before each step
  --dry-run                    Print planned actions without invoking codex
  --include-guardrails         Also include unchecked tasks under "## Guardrails"
  -h, --help                   Show this help

Example:
  scripts/ralph-loop.sh --task-file docs/specs/2026-03-01-single-evm-steward-tasklist.md --auto
USAGE
}

while (($# > 0)); do
  case "$1" in
    --task-file)
      TASK_FILE="$2"
      shift 2
      ;;
    --max-steps)
      MAX_STEPS="$2"
      shift 2
      ;;
    --model)
      MODEL="$2"
      shift 2
      ;;
    --runner)
      RUNNER="$2"
      shift 2
      ;;
    --auto)
      AUTO_APPROVE=1
      shift
      ;;
    --dry-run)
      DRY_RUN=1
      shift
      ;;
    --include-guardrails)
      INCLUDE_GUARDRAILS=1
      shift
      ;;
    -h|--help)
      usage
      exit 0
      ;;
    *)
      echo "Unknown option: $1" >&2
      usage >&2
      exit 1
      ;;
  esac
done

if [[ "${TASK_FILE}" != /* ]]; then
  TASK_FILE="${REPO_ROOT}/${TASK_FILE#./}"
fi

if [[ ! -f "${TASK_FILE}" ]]; then
  echo "Task file not found: ${TASK_FILE}" >&2
  exit 1
fi

if [[ "${RUNNER}" != "auto" && "${RUNNER}" != "codex" ]]; then
  echo "Invalid --runner value: ${RUNNER}" >&2
  exit 1
fi

if [[ "${RUNNER}" == "auto" ]]; then
  RUNNER="codex"
fi

if ! command -v "${RUNNER}" >/dev/null 2>&1; then
  echo "${RUNNER} CLI not found in PATH" >&2
  exit 1
fi

mkdir -p "${LOG_DIR}"

ensure_important_lessons_section() {
  if grep -q '^## Important Lessons' "${TASK_FILE}"; then
    return 0
  fi

  cat >>"${TASK_FILE}" <<'EOF'

## Important Lessons

- Keep this section current with durable implementation lessons discovered while executing tasks.
EOF
}

extract_important_lessons() {
  local lessons
  lessons="$(
    awk '
      BEGIN { in_lessons = 0 }
      /^## Important Lessons$/ { in_lessons = 1; next }
      in_lessons && /^## / { exit }
      in_lessons { print }
    ' "${TASK_FILE}"
  )"

  if [[ -z "${lessons// }" ]]; then
    printf '%s\n' "- (none yet)"
  else
    printf '%s\n' "${lessons}"
  fi
}

extract_first_unchecked_task() {
  awk -v include_guardrails="${INCLUDE_GUARDRAILS}" '
    BEGIN {
      in_code = 0
      section = ""
      active = 0
    }
    /^```/ {
      in_code = !in_code
      next
    }
    in_code {
      next
    }
    /^## / {
      section = $0
      if (section ~ /^## Phase / || section ~ /^## Final validation gate/) {
        active = 1
      } else if (include_guardrails == "1" && section ~ /^## Guardrails /) {
        active = 1
      } else {
        active = 0
      }
      next
    }
    active && $0 ~ /^[[:space:]]*-[[:space:]]\[[[:space:]]\][[:space:]]+/ {
      task_text = $0
      sub(/^[[:space:]]*-[[:space:]]\[[[:space:]]\][[:space:]]+/, "", task_text)
      gsub(/[[:space:]]+$/, "", task_text)
      print NR "\t" section "\t" task_text
      exit
    }
  ' "${TASK_FILE}"
}

build_prompt() {
  local line_no="$1"
  local section="$2"
  local task_text="$3"
  local lessons="$4"
  cat <<EOF
Execute exactly one checklist item from ${TASK_FILE} with fresh context.

Target item:
- File: ${TASK_FILE}
- Section: ${section}
- Line: ${line_no}
- Checkbox: [ ] ${task_text}

Requirements:
1. Follow AGENTS.md instructions in this repository.
2. Complete only this one item; do not implement future items.
3. Run the minimum targeted validation for this item.
4. Read and apply the "Important Lessons" section before editing.
5. If blocked, keep [ ] and add a short "Blocker:" note under the item.
6. If this step yields a durable lesson, update the "Important Lessons" section.
7. Return a concise summary including files changed and validation run.

Important Lessons:
${lessons}
EOF
}

task_state() {
  local section="$1"
  local task_text="$2"
  awk -v section_target="${section}" -v task_target="${task_text}" '
    BEGIN { in_section = 0 }
    /^## / {
      in_section = ($0 == section_target)
    }
    in_section && $0 ~ /^[[:space:]]*-[[:space:]]\[[ x]\][[:space:]]+/ {
      line = $0
      text = line
      sub(/^[[:space:]]*-[[:space:]]\[[ x]\][[:space:]]+/, "", text)
      gsub(/[[:space:]]+$/, "", text)
      if (text == task_target) {
        if (line ~ /^[[:space:]]*-[[:space:]]\[x\][[:space:]]+/) {
          print "checked"
        } else {
          print "unchecked"
        }
        exit
      }
    }
    END {
      if (NR >= 0 && !length($0)) {
        # no-op; keeps awk happy for empty files
      }
    }
  ' "${TASK_FILE}"
}

task_has_blocker() {
  local section="$1"
  local task_text="$2"
  awk -v section_target="${section}" -v task_target="${task_text}" '
    BEGIN { in_section = 0; in_task = 0 }
    /^## / {
      in_section = ($0 == section_target)
      if (!in_section) in_task = 0
    }
    !in_section { next }
    $0 ~ /^[[:space:]]*-[[:space:]]\[[ x]\][[:space:]]+/ {
      line = $0
      text = line
      sub(/^[[:space:]]*-[[:space:]]\[[ x]\][[:space:]]+/, "", text)
      gsub(/[[:space:]]+$/, "", text)
      if (text == task_target) {
        in_task = 1
        next
      }
      if (in_task) {
        in_task = 0
      }
    }
    in_task && $0 ~ /Blocker:/ {
      print "yes"
      exit
    }
  ' "${TASK_FILE}"
}

mark_task_completed() {
  local section="$1"
  local task_text="$2"
  local tmp_file
  tmp_file="$(mktemp)"

  awk -v section_target="${section}" -v task_target="${task_text}" '
    BEGIN { in_section = 0; done = 0 }
    /^## / {
      in_section = ($0 == section_target)
      print
      next
    }
    {
      if (in_section && !done && $0 ~ /^[[:space:]]*-[[:space:]]\[[[:space:]]\][[:space:]]+/) {
        line = $0
        text = line
        sub(/^[[:space:]]*-[[:space:]]\[[[:space:]]\][[:space:]]+/, "", text)
        gsub(/[[:space:]]+$/, "", text)
        if (text == task_target) {
          sub(/\[[[:space:]]\]/, "[x]", line)
          print line
          done = 1
          next
        }
      }
      print
    }
  ' "${TASK_FILE}" >"${tmp_file}"

  mv "${tmp_file}" "${TASK_FILE}"
}

run_codex_exec() {
  local prompt="$1"
  local label="$2"
  local prompt_file="${LOG_DIR}/${label}-prompt.txt"

  printf '%s\n' "${prompt}" > "${prompt_file}"

  if ((DRY_RUN == 1)); then
    echo "--- prompt preview (${label}) ---"
    cat "${prompt_file}"
    echo "--- end preview ---"
    return 0
  fi

  local cmd=(codex exec --ephemeral --full-auto -C "${REPO_ROOT}")
  if [[ -n "${MODEL}" ]]; then
    cmd+=(-m "${MODEL}")
  fi
  cmd+=("${prompt}")
  "${cmd[@]}"
}

run_final_review() {
  local lessons
  lessons="$(extract_important_lessons)"
  local prompt
  prompt="$(cat <<EOF
All checklist steps are complete in ${TASK_FILE}.

Run a final review of all current uncommitted changes for this tasklist work and do the following:
1. Identify functional gaps and unnecessary complexity.
2. Identify duplications (logic, tests, docs, APIs) and remove or consolidate them where safe.
3. Identify inconsistencies (naming, behavior, docs vs code, API shapes, validation expectations) and resolve them.
4. Simplify implementation where safe and beneficial.
5. Run targeted validations for any new edits.
6. Update "## Important Lessons" in ${TASK_FILE} with final durable lessons.
7. Provide a concise review summary with files changed, validations run, remaining risks, and any intentional follow-ups.

Current Important Lessons:
${lessons}
EOF
)"
  run_codex_exec "${prompt}" "final-review"
}

step=0
unchanged_count=0

ensure_important_lessons_section

while ((step < MAX_STEPS)); do
  next_task="$(extract_first_unchecked_task || true)"
  if [[ -z "${next_task}" ]]; then
    echo "No unchecked tasks found in selected sections."
    if ((DRY_RUN == 1)); then
      echo "Dry-run: would execute final review pass."
      exit 0
    fi
    if ((AUTO_APPROVE == 0)); then
      read -r -p "Run final review for gaps/simplification now? [Y/n] " final_answer
      final_answer="${final_answer:-Y}"
      case "${final_answer}" in
        Y|y) run_final_review ;;
        N|n) ;;
        *) echo "Invalid choice, skipping final review." ;;
      esac
    else
      run_final_review
    fi
    exit 0
  fi

  IFS=$'\t' read -r line_no section task_text <<<"${next_task}"
  step=$((step + 1))

  echo
  echo "Step ${step}/${MAX_STEPS}"
  echo "Task line ${line_no}: ${task_text}"
  echo "Section: ${section}"

  if ((AUTO_APPROVE == 0 && DRY_RUN == 0)); then
    read -r -p "Run this step? [Y/n/q] " answer
    answer="${answer:-Y}"
    case "${answer}" in
      Y|y) ;;
      N|n)
        echo "Skipped by user."
        continue
        ;;
      Q|q)
        echo "Stopped by user."
        exit 0
        ;;
      *)
        echo "Invalid choice, stopping."
        exit 1
        ;;
    esac
  fi

  lessons="$(extract_important_lessons)"
  prompt="$(build_prompt "${line_no}" "${section}" "${task_text}" "${lessons}")"
  run_codex_exec "${prompt}" "step-${step}"

  state_after="$(task_state "${section}" "${task_text}" || true)"
  if [[ "${state_after}" == "unchecked" ]]; then
    blocker_after="$(task_has_blocker "${section}" "${task_text}" || true)"
    if [[ "${blocker_after}" == "yes" ]]; then
      echo "Task still blocked; checkbox left unchecked."
    else
      mark_now="Y"
      if ((AUTO_APPROVE == 0)); then
        read -r -p "Mark this step as completed now? [Y/n] " mark_answer
        mark_now="${mark_answer:-Y}"
      fi
      case "${mark_now}" in
        Y|y)
          mark_task_completed "${section}" "${task_text}"
          echo "Marked step complete in tasklist."
          ;;
        *)
          echo "Left step unchecked."
          ;;
      esac
    fi
  fi

  remaining_after="$(extract_first_unchecked_task || true)"
  if [[ "${remaining_after}" == "${next_task}" ]]; then
    unchanged_count=$((unchanged_count + 1))
    echo "Warning: first unchecked task did not change after step ${step}."
  else
    unchanged_count=0
  fi

  if ((unchanged_count >= 3)); then
    echo "Stopping: same top task remained unchecked for 3 consecutive runs."
    exit 2
  fi
done

echo "Stopped: reached max steps (${MAX_STEPS})."
