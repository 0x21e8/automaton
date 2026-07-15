import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { isAbsolute, join, normalize } from "node:path";

import {
  formatEvaluationExperimentError,
  parseEvaluationExperimentYaml,
  type EvaluationExperiment
} from "@ic-automaton/shared";

import type { ExperimentFile } from "../types.js";
import { parseGenerationScenario } from "./generation-scenario.js";
import type { GenerationScenario } from "./generation-scenario.js";

export interface LoadedExperiment extends ExperimentFile {
  parsed: EvaluationExperiment;
  generationScenario: GenerationScenario | null;
}

export async function loadExperimentFile(
  repoRoot: string,
  experimentPath: string
): Promise<LoadedExperiment> {
  const absolutePath = isAbsolute(experimentPath)
    ? normalize(experimentPath)
    : normalize(join(repoRoot, experimentPath));
  const source = await readFile(absolutePath, "utf8");

  try {
    let generationScenario = null;
    const scenarioLine = source.match(/^generationScenarioJson:\s*'(.+)'\s*$/m);
    const evaluationSource = scenarioLine === null ? source : source.replace(scenarioLine[0], "");
    if (scenarioLine !== null) {
      generationScenario = parseGenerationScenario(JSON.parse(scenarioLine[1] ?? "null"));
    }
    const parsed = parseEvaluationExperimentYaml(evaluationSource);
    const hash = createHash("sha256").update(source).digest("hex");

    return {
      path: isAbsolute(experimentPath) ? absolutePath : normalize(experimentPath),
      absolutePath,
      source,
      hash,
      parsed,
      generationScenario
    };
  } catch (error) {
    throw new Error(
      `Invalid evaluation experiment at ${experimentPath}:\n${formatEvaluationExperimentError(error)}`
    );
  }
}
