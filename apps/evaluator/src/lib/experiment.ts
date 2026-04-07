import { createHash } from "node:crypto";
import { readFile } from "node:fs/promises";
import { isAbsolute, join, normalize } from "node:path";

import {
  formatEvaluationExperimentError,
  parseEvaluationExperimentYaml,
  type EvaluationExperiment
} from "@ic-automaton/shared";

import type { ExperimentFile } from "../types.js";

export interface LoadedExperiment extends ExperimentFile {
  parsed: EvaluationExperiment;
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
    const parsed = parseEvaluationExperimentYaml(source);
    const hash = createHash("sha256").update(source).digest("hex");

    return {
      path: isAbsolute(experimentPath) ? absolutePath : normalize(experimentPath),
      absolutePath,
      source,
      hash,
      parsed
    };
  } catch (error) {
    throw new Error(
      `Invalid evaluation experiment at ${experimentPath}:\n${formatEvaluationExperimentError(error)}`
    );
  }
}
