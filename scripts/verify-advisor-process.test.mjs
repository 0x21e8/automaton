import assert from "node:assert/strict";
import fs from "node:fs";
import path from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { validateAdvisorProcess } from "./verify-advisor-process.mjs";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const sources = () => ({
  executorSource: fs.readFileSync(path.join(root, ".codex/agents/advisor-plan-executor.toml"), "utf8"),
  reviewerSource: fs.readFileSync(path.join(root, ".codex/agents/best-practice-reviewer.toml"), "utf8"),
  agentsSource: fs.readFileSync(path.join(root, "AGENTS.md"), "utf8")
});

test("tracked advisor profiles use the required models, effort, and safety contracts", () => {
  const result = validateAdvisorProcess(sources());
  assert.deepEqual(result.errors, []);
  assert.deepEqual(result.executorFields, {
    name: "advisor-plan-executor",
    model: "gpt-5.3-codex-spark",
    effort: "high",
    sandbox: "workspace-write"
  });
  assert.deepEqual(result.reviewerFields, {
    name: "best-practice-reviewer",
    model: "gpt-5.6-sol",
    effort: "medium",
    sandbox: "read-only"
  });
});

test("validation rejects downgraded reasoning and removed caller-contract checks", () => {
  const fixture = sources();
  fixture.executorSource = fixture.executorSource
    .replace('model_reasoning_effort = "high"', 'model_reasoning_effort = "low"')
    .replaceAll("caller contract", "call handling");
  fixture.reviewerSource = fixture.reviewerSource
    .replace('model_reasoning_effort = "medium"', 'model_reasoning_effort = "low"')
    .replaceAll("enumerate every caller", "inspect some callers");

  const result = validateAdvisorProcess(fixture);
  assert.ok(result.errors.includes("executor reasoning effort must be high"));
  assert.ok(result.errors.includes("executor must enforce caller contracts"));
  assert.ok(result.errors.includes("reviewer reasoning effort must be medium"));
  assert.ok(result.errors.includes("reviewer must enumerate helper callers"));
});

test("validation rejects a generic executor model and writable reviewer", () => {
  const fixture = sources();
  fixture.executorSource = fixture.executorSource.replace("gpt-5.3-codex-spark", "gpt-generic");
  fixture.reviewerSource = fixture.reviewerSource.replace('sandbox_mode = "read-only"', 'sandbox_mode = "workspace-write"');

  const result = validateAdvisorProcess(fixture);
  assert.ok(result.errors.includes("executor model must be gpt-5.3-codex-spark"));
  assert.ok(result.errors.includes("reviewer sandbox must be read-only"));
});
