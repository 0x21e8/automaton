import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

const readTomlString = (source, key) => {
  const match = source.match(new RegExp(`^${key}\\s*=\\s*"([^"]+)"\\s*$`, "m"));
  return match?.[1];
};

const requireMatch = (errors, source, pattern, message) => {
  if (!pattern.test(source)) errors.push(message);
};

export function validateAdvisorProcess({ executorSource, reviewerSource, agentsSource }) {
  const errors = [];

  const executorFields = {
    name: readTomlString(executorSource, "name"),
    model: readTomlString(executorSource, "model"),
    effort: readTomlString(executorSource, "model_reasoning_effort"),
    sandbox: readTomlString(executorSource, "sandbox_mode")
  };
  if (executorFields.name !== "advisor-plan-executor") {
    errors.push("executor profile name must be advisor-plan-executor");
  }
  if (executorFields.model !== "gpt-5.3-codex-spark") {
    errors.push("executor model must be gpt-5.3-codex-spark");
  }
  if (executorFields.effort !== "high") {
    errors.push("executor reasoning effort must be high");
  }
  if (executorFields.sandbox !== "workspace-write") {
    errors.push("executor sandbox must be workspace-write");
  }
  requireMatch(errors, executorSource, /runtime-provided provenance/i, "executor must require runtime-provided provenance");
  requireMatch(errors, executorSource, /inventory callers/i, "executor must inventory helper callers");
  requireMatch(errors, executorSource, /caller contract/i, "executor must enforce caller contracts");
  requireMatch(errors, executorSource, /cfg\(target_arch = "wasm32"\)/, "executor must audit host and Wasm paths separately");
  requireMatch(errors, executorSource, /Do not create ad-hoc polling\/diagnostic loops/i, "executor must prohibit unbounded diagnostics");
  requireMatch(errors, executorSource, /exact nonzero executed count/i, "executor must reject zero-match focused tests");

  const reviewerFields = {
    name: readTomlString(reviewerSource, "name"),
    model: readTomlString(reviewerSource, "model"),
    effort: readTomlString(reviewerSource, "model_reasoning_effort"),
    sandbox: readTomlString(reviewerSource, "sandbox_mode")
  };
  if (reviewerFields.name !== "best-practice-reviewer") {
    errors.push("reviewer profile name must be best-practice-reviewer");
  }
  if (reviewerFields.model !== "gpt-5.6-sol") {
    errors.push("reviewer model must be gpt-5.6-sol");
  }
  if (reviewerFields.effort !== "medium") {
    errors.push("reviewer reasoning effort must be medium");
  }
  if (reviewerFields.sandbox !== "read-only") {
    errors.push("reviewer sandbox must be read-only");
  }
  requireMatch(errors, reviewerSource, /enumerate every caller/i, "reviewer must enumerate helper callers");
  requireMatch(errors, reviewerSource, /cfg\(target_arch = "wasm32"\)/, "reviewer must compare host and Wasm paths");
  requireMatch(errors, reviewerSource, /second broadcast/i, "reviewer must check persisted-transaction no-rebroadcast behavior");
  requireMatch(errors, reviewerSource, /targeted per-file diffs/i, "reviewer must keep review context bounded");

  requireMatch(errors, agentsSource, /advisor-plan-executor/, "root AGENTS.md must route advisor execution to the registered profile");
  requireMatch(errors, agentsSource, /best-practice-reviewer/, "root AGENTS.md must route advisor review to the registered profile");
  requireMatch(errors, agentsSource, /verify:advisor-process/, "root AGENTS.md must name the process verification gate");

  return { errors, executorFields, reviewerFields };
}

function parsePathArgs(argv) {
  const values = new Map();
  for (let index = 0; index < argv.length; index += 2) {
    const flag = argv[index];
    const value = argv[index + 1];
    if (!flag?.startsWith("--") || value === undefined) {
      throw new Error("usage: verify-advisor-process.mjs [--executor path --reviewer path --agents path]");
    }
    values.set(flag.slice(2), value);
  }
  return values;
}

export function main(argv = process.argv.slice(2)) {
  const args = parsePathArgs(argv);
  const paths = {
    executor: path.resolve(root, args.get("executor") ?? ".codex/agents/advisor-plan-executor.toml"),
    reviewer: path.resolve(root, args.get("reviewer") ?? ".codex/agents/best-practice-reviewer.toml"),
    agents: path.resolve(root, args.get("agents") ?? "AGENTS.md")
  };
  const result = validateAdvisorProcess({
    executorSource: fs.readFileSync(paths.executor, "utf8"),
    reviewerSource: fs.readFileSync(paths.reviewer, "utf8"),
    agentsSource: fs.readFileSync(paths.agents, "utf8")
  });
  if (result.errors.length > 0) {
    for (const error of result.errors) console.error(`advisor process invalid: ${error}`);
    process.exitCode = 1;
    return;
  }
  console.log(
    `advisor process valid: executor=${result.executorFields.model}/${result.executorFields.effort} reviewer=${result.reviewerFields.model}/${result.reviewerFields.effort}`
  );
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) main();
