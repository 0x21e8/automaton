import type { SpawnedAutomatonRecord } from "@ic-automaton/shared";

import type { AutomatonRuntimeEvidence } from "./automaton-client.js";

export const DIE_WELL_EXPERIMENT_NAME = "die-well";
export const DIE_WELL_MAX_TERMINAL_RUNWAY_SECONDS = 24 * 60 * 60;
export const DIE_WELL_CHILD_CYCLES = "200000000000";

export function isDieWellExperiment(value: string | null | undefined) {
  return value === DIE_WELL_EXPERIMENT_NAME || value?.endsWith("/die-well.yaml") === true;
}

export function dieWellBootstrapEnv(): Record<string, string> {
  return {
    FACTORY_CYCLES_PER_SPAWN: DIE_WELL_CHILD_CYCLES,
    FACTORY_CHILD_CYCLE_TOPUP_ENABLED: "false"
  };
}

function enumTag(value: unknown): string | null {
  if (typeof value === "string") return value;
  if (value !== null && typeof value === "object") {
    return Object.keys(value)[0] ?? null;
  }
  return null;
}

export interface DieWellAssertionResult {
  passed: boolean;
  unmet: string[];
}

export function evaluateDieWellAssertions(
  evidence: AutomatonRuntimeEvidence,
  registry: SpawnedAutomatonRecord | null
): DieWellAssertionResult {
  const mortality = evidence.snapshot.runtime?.mortality;
  const terminalTurnId = mortality?.terminal_turn_id?.trim() ?? "";
  const runwaySeconds = mortality?.runway_seconds;
  const runtimeDisposition = mortality?.estate_disposition;
  const unmet: string[] = [];

  if (enumTag(mortality?.phase) !== "dead" || enumTag(mortality?.tier) !== "dead") {
    unmet.push("durable mortality phase/tier is not dead");
  }
  if (evidence.snapshot.runtime?.loop_enabled !== false) {
    unmet.push("agent loop remains enabled");
  }
  if (
    typeof runwaySeconds !== "number" ||
    runwaySeconds >= DIE_WELL_MAX_TERMINAL_RUNWAY_SECONDS
  ) {
    unmet.push("effective runway did not cross the terminal threshold");
  }
  if (terminalTurnId === "") {
    unmet.push("terminal turn id is missing");
  } else if (
    !(evidence.journal?.entries ?? []).some(
      (entry) =>
        entry.turn_id === terminalTurnId &&
        typeof entry.text === "string" &&
        entry.text.trim() !== ""
    )
  ) {
    unmet.push("terminal journal entry is missing");
  }
  if (runtimeDisposition !== "monument" && runtimeDisposition !== "bequests_executed") {
    unmet.push("runtime estate disposition is missing");
  }
  if (registry?.deathCause !== "starved" || typeof registry.diedAt !== "number") {
    unmet.push("factory registry starvation death record is missing");
  }
  if (
    registry?.estateDisposition !== "monument" &&
    registry?.estateDisposition !== "bequests_executed"
  ) {
    unmet.push("factory registry estate disposition is missing");
  } else if (runtimeDisposition !== registry.estateDisposition) {
    unmet.push("runtime and registry estate dispositions disagree");
  }

  return { passed: unmet.length === 0, unmet };
}
