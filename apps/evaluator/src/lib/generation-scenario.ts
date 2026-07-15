export type GenerationScenarioStep =
  | { action: "seed_earnings"; automatonId: string; amountRaw: string }
  | { action: "advance_time"; durationMs: number }
  | { action: "reproduce"; parentId: string; childId: string }
  | { action: "starve"; automatonId: string };

export interface GenerationScenario {
  driver: "pocketic";
  steps: GenerationScenarioStep[];
  assertions: { descendantCreated: boolean; lineageMetricsPopulated: boolean; starvationRecorded: boolean; descendantSurvived: boolean; inheritanceVerified: boolean };
}

export interface InheritanceEvidence {
  parentCanisterId: string;
  childCanisterId: string;
  parentConstitutionHash: string;
  childConstitutionHash: string;
  childRecordedParentHash: string;
  generation: number;
  memoryKey: string;
  memoryValue: string;
  inheritedMemoryKey: string;
  inheritedSourceTag: string;
  constitutionDiff: string[];
}

export interface SeedEarningsEvidence {
  canisterId: string;
  evmAddress: string;
  indexedBefore: string;
  chainBefore: string;
  amountRaw: string;
  chainAfter: string;
  indexedAfter: string;
}

export interface LineageMetrics {
  descendantCount: number;
  generationDepth: number;
  starvedCanisterIds: string[];
  survivingDescendantIds: string[];
  inheritance: InheritanceEvidence[];
  earningsSeeds: SeedEarningsEvidence[];
}

export interface GenerationScenarioDriver {
  seedEarnings(canisterId: string, amountRaw: string): Promise<void>;
  advanceTime(durationMs: number): Promise<void>;
  reproduce(parentCanisterId: string, childId: string): Promise<{ childCanisterId: string }>;
  starve(canisterId: string): Promise<void>;
  readLineageMetrics(): Promise<LineageMetrics>;
}

export interface GenerationScenarioResult {
  descendants: Map<string, string>;
  lineageMetrics: LineageMetrics;
}

export function parseGenerationScenario(value: unknown): GenerationScenario {
  if (typeof value !== "object" || value === null || Array.isArray(value)) {
    throw new Error("generationScenario must be a JSON object");
  }
  const record = value as Record<string, unknown>;
  if (record.driver !== "pocketic" || !Array.isArray(record.steps)) {
    throw new Error("generationScenario requires driver=pocketic and a steps array");
  }
  const steps = record.steps.map((step, index) => {
    if (typeof step !== "object" || step === null || Array.isArray(step)) {
      throw new Error(`generationScenario.steps[${index}] must be an object`);
    }
    const item = step as Record<string, unknown>;
    switch (item.action) {
      case "seed_earnings":
        if (typeof item.automatonId !== "string" || typeof item.amountRaw !== "string" || !/^[1-9][0-9]*$/.test(item.amountRaw)) throw new Error(`invalid seed_earnings step ${index}`);
        return { action: item.action, automatonId: item.automatonId, amountRaw: item.amountRaw } as const;
      case "advance_time":
        if (!Number.isSafeInteger(item.durationMs) || Number(item.durationMs) <= 0) throw new Error(`invalid advance_time step ${index}`);
        return { action: item.action, durationMs: Number(item.durationMs) } as const;
      case "reproduce":
        if (typeof item.parentId !== "string" || typeof item.childId !== "string") throw new Error(`invalid reproduce step ${index}`);
        return { action: item.action, parentId: item.parentId, childId: item.childId } as const;
      case "starve":
        if (typeof item.automatonId !== "string") throw new Error(`invalid starve step ${index}`);
        return { action: item.action, automatonId: item.automatonId } as const;
      default:
        throw new Error(`unsupported generationScenario action at step ${index}`);
    }
  });
  const assertions = record.assertions as Record<string, unknown> | undefined;
  if (assertions?.descendantCreated !== true || assertions.lineageMetricsPopulated !== true || assertions.starvationRecorded !== true || assertions.descendantSurvived !== true || assertions.inheritanceVerified !== true) {
    throw new Error("generationScenario must assert descendant creation, lineage metrics, starvation, descendant survival, and inheritance");
  }
  return { driver: "pocketic", steps, assertions: { descendantCreated: true, lineageMetricsPopulated: true, starvationRecorded: true, descendantSurvived: true, inheritanceVerified: true } };
}

export async function executeGenerationScenario(
  scenario: GenerationScenario,
  driver: GenerationScenarioDriver,
  fleet: ReadonlyMap<string, string>
): Promise<GenerationScenarioResult> {
  const identities = new Map(fleet);
  const descendants = new Map<string, string>();
  for (const step of scenario.steps) {
    if (step.action === "advance_time") {
      await driver.advanceTime(step.durationMs);
      continue;
    }
    const id = step.action === "reproduce" ? step.parentId : step.automatonId;
    const canisterId = identities.get(id);
    if (canisterId === undefined) throw new Error(`generation scenario references unknown automaton ${id}`);
    if (step.action === "seed_earnings") await driver.seedEarnings(canisterId, step.amountRaw);
    if (step.action === "starve") await driver.starve(canisterId);
    if (step.action === "reproduce") {
      const child = await driver.reproduce(canisterId, step.childId);
      if (child.childCanisterId.trim() === "") throw new Error("reproduction did not return a child canister");
      identities.set(step.childId, child.childCanisterId);
      descendants.set(step.childId, child.childCanisterId);
    }
  }
  const lineageMetrics = await driver.readLineageMetrics();
  if (scenario.assertions.descendantCreated && descendants.size === 0) throw new Error("generation assertion failed: no descendant was created");
  if (scenario.assertions.lineageMetricsPopulated && (lineageMetrics.descendantCount < 1 || lineageMetrics.generationDepth < 1)) {
    throw new Error("generation assertion failed: lineage metrics were not populated");
  }
  if (scenario.assertions.starvationRecorded && lineageMetrics.starvedCanisterIds.length < 1) throw new Error("generation assertion failed: starvation was not recorded");
  if (scenario.assertions.descendantSurvived && lineageMetrics.survivingDescendantIds.length < 1) throw new Error("generation assertion failed: no descendant survived its parent");
  if (scenario.assertions.inheritanceVerified && lineageMetrics.inheritance.length < 1) throw new Error("generation assertion failed: constitution and memory inheritance were not verified");
  return { descendants, lineageMetrics };
}
