import { mkdir, readFile, readdir, writeFile } from "node:fs/promises";
import { join } from "node:path";

import type {
  EvaluationAutomatonEvidenceSample,
  EvaluationRunHistoryResponse,
  EvaluationRunListItem,
  EvaluationRunSummary
} from "@ic-automaton/shared";

import type {
  EvaluationArtifacts,
  EvaluationTimelineEvent,
  StoredManifest
} from "../types.js";

export class ArtifactStore {
  constructor(private readonly rootDirectory: string) {}

  async createRunArtifacts(runId: string): Promise<EvaluationArtifacts> {
    const runDirectory = join(this.rootDirectory, runId);
    const samplesDirectory = join(runDirectory, "samples");

    await mkdir(samplesDirectory, { recursive: true });

    return {
      runDirectory,
      manifestPath: join(runDirectory, "manifest.json"),
      eventsPath: join(runDirectory, "events.ndjson"),
      samplesDirectory,
      summaryPath: join(runDirectory, "summary.json"),
      reportPath: join(runDirectory, "report.md")
    };
  }

  async writeManifest(artifacts: EvaluationArtifacts, manifest: StoredManifest) {
    await writeFile(artifacts.manifestPath, `${JSON.stringify(manifest, null, 2)}\n`, "utf8");
  }

  async appendEvent(artifacts: EvaluationArtifacts, event: EvaluationTimelineEvent) {
    await writeFile(artifacts.eventsPath, `${JSON.stringify(event)}\n`, {
      encoding: "utf8",
      flag: "a"
    });
  }

  async appendSample(
    artifacts: EvaluationArtifacts,
    automatonId: string,
    sample: EvaluationAutomatonEvidenceSample
  ) {
    const filePath = join(artifacts.samplesDirectory, `${automatonId}.jsonl`);
    await writeFile(filePath, `${JSON.stringify(sample)}\n`, {
      encoding: "utf8",
      flag: "a"
    });
  }

  async writeSummary(artifacts: EvaluationArtifacts, summary: EvaluationRunSummary) {
    await writeFile(artifacts.summaryPath, `${JSON.stringify(summary, null, 2)}\n`, "utf8");
  }

  async writeReport(artifacts: EvaluationArtifacts, reportMarkdown: string) {
    await writeFile(artifacts.reportPath, `${reportMarkdown.trimEnd()}\n`, "utf8");
  }

  async listHistoricalRuns(): Promise<EvaluationRunHistoryResponse> {
    await mkdir(this.rootDirectory, { recursive: true });
    const directoryEntries = await readdir(this.rootDirectory, {
      withFileTypes: true
    });
    const runs: EvaluationRunListItem[] = [];

    for (const entry of directoryEntries) {
      if (!entry.isDirectory()) {
        continue;
      }

      const manifestPath = join(this.rootDirectory, entry.name, "manifest.json");

      try {
        const manifest = JSON.parse(await readFile(manifestPath, "utf8")) as StoredManifest;
        runs.push({
          runId: manifest.run.runId,
          experimentPath: manifest.run.experimentPath,
          startedAt: manifest.run.startedAt,
          endedAt: manifest.run.endedAt,
          runState: manifest.run.runState,
          completionReason: manifest.completionReason
        });
      } catch {}
    }

    runs.sort((left, right) => right.startedAt - left.startedAt);
    return { runs };
  }
}
