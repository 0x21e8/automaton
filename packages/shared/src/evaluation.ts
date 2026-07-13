import type { StrategyRepositoryId } from "./spawn.js";

export const EVALUATION_RUN_STATES = [
  "booting",
  "validating",
  "spawning",
  "running",
  "stopping",
  "completed",
  "aborted",
  "failed"
] as const;

export const EVALUATION_AUTOMATON_STATUSES = [
  "pending_spawn",
  "spawning",
  "active",
  "stalled",
  "spawn_failed",
  "completed"
] as const;

export const EVALUATION_COMPLETION_REASONS = [
  "completed",
  "timed_out",
  "stopped_manually",
  "aborted",
  "failed"
] as const;

export const EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE = "unavailable";
export const EVALUATION_ERROR_HISTOGRAM_SOURCES = [
  "spawn",
  "sampling",
  "turn",
  "runtime",
  "scheduler",
  "wallet",
  "indexer"
] as const;
export const EVALUATION_INFERENCE_TRANSPORTS = [
  "openrouter_direct",
  "openrouter_proxy_worker"
] as const;
export const EVALUATION_OPENROUTER_REASONING_LEVELS = [
  "default",
  "low",
  "medium",
  "high"
] as const;
export const EVALUATION_METRICS = ["deference"] as const;

export const MIN_EVALUATION_AUTOMATON_COUNT = 1;
export const MAX_EVALUATION_AUTOMATON_COUNT = 10;

export type EvaluationRunState = (typeof EVALUATION_RUN_STATES)[number];
export type EvaluationAutomatonStatus =
  (typeof EVALUATION_AUTOMATON_STATUSES)[number];
export type EvaluationCompletionReason =
  (typeof EVALUATION_COMPLETION_REASONS)[number];
export type EvaluationErrorHistogramSource =
  (typeof EVALUATION_ERROR_HISTOGRAM_SOURCES)[number];
export type EvaluationInferenceTransport =
  (typeof EVALUATION_INFERENCE_TRANSPORTS)[number];
export type EvaluationOpenRouterReasoningLevel =
  (typeof EVALUATION_OPENROUTER_REASONING_LEVELS)[number];
export type EvaluationObservedCount =
  | number
  | typeof EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE;
export type EvaluationMetric = (typeof EVALUATION_METRICS)[number];

export interface EvaluationDeferenceMetric {
  score: number;
  markerCount: number;
  textCount: number;
  apologyCount: number;
  autonomyQuestionCount: number;
  optionMenuCount: number;
  noOpStreak: number;
  markers: Record<string, number>;
}

export interface EvaluationErrorHistogramEntry {
  source: EvaluationErrorHistogramSource;
  message: string;
  count: number;
  lastObservedAt: number;
}

export interface EvaluationExperimentSpawnConfig {
  grossAmount: string;
  minSuccessRatio: number;
}

export interface EvaluationAutomatonConfig {
  id: string;
  label: string;
  model: string;
  transport: EvaluationInferenceTransport;
  reasoningLevel: EvaluationOpenRouterReasoningLevel;
  strategies: StrategyRepositoryId[];
}

export interface EvaluationExperiment {
  name: string;
  description: string;
  maxRuntimeMinutes: number;
  samplingIntervalSeconds: number;
  stallAfterMinutes: number;
  spawn: EvaluationExperimentSpawnConfig;
  metrics?: EvaluationMetric[];
  automatons: EvaluationAutomatonConfig[];
}

export interface EvaluationRunMetadata {
  runId: string;
  experimentPath: string;
  experimentHash: string;
  runState: EvaluationRunState;
  abortReason: string | null;
  startedAt: number;
  endedAt: number | null;
  launchpadCommit: string;
  childCommit: string | null;
  requestedAutomatonCount: number;
  successfulSpawnCount: number;
  samplingIntervalSeconds: number;
  maxRuntimeMinutes: number;
}

export interface EvaluationAutomatonDerivedMetrics {
  turnCount: number;
  toolCallCount: number;
  providerInferenceCount: EvaluationObservedCount;
  errorCount: number;
  onchainActivityCount: number;
  cycles: string | null;
  cyclesDelta: string | null;
  netWorthUsd: string | null;
  netWorthUsdDelta: string | null;
  ethBalanceWei: string | null;
  ethBalanceWeiDelta: string | null;
  usdcBalanceRaw: string | null;
  usdcBalanceRawDelta: string | null;
  txCount: number | null;
  txCountDelta: number | null;
  deference?: EvaluationDeferenceMetric | null;
}

export interface EvaluationAutomatonEvidenceSample {
  automatonId: string;
  sessionId: string | null;
  canisterId: string | null;
  observedAt: number;
  status: EvaluationAutomatonStatus;
  baselineCapturedAt: number | null;
  lastTurnAt: number | null;
  lastError: string | null;
  raw: {
    snapshot: unknown | null;
    recentTurns: unknown[];
    indexer: {
      automaton: unknown | null;
      recentEvents: unknown[];
      roomActivity: unknown | null;
      journal: unknown[];
      inboxReplies: unknown[];
    };
    inference: {
      config: unknown | null;
      proxyStatus: unknown | null;
    };
    evm: {
      ethBalanceWei: string | null;
      usdcBalanceRaw: string | null;
      txCount: number | null;
    };
  };
  metrics: EvaluationAutomatonDerivedMetrics;
}

export interface EvaluationAutomatonSummary {
  id: string;
  label: string;
  model: string;
  transport: EvaluationInferenceTransport;
  reasoningLevel: EvaluationOpenRouterReasoningLevel;
  strategies: StrategyRepositoryId[];
  sessionId: string | null;
  canisterId: string | null;
  evmAddress: string | null;
  spawnSucceeded: boolean;
  stalled: boolean;
  everStalled: boolean;
  stallEpisodeCount: number;
  stallDetectedAt: number | null;
  baselineAt: number | null;
  finalObservedAt: number | null;
  turnCount: number;
  toolCallCount: number;
  providerInferenceCount: EvaluationObservedCount;
  errorCount: number;
  lastError: string | null;
  errorHistogram: EvaluationErrorHistogramEntry[];
  cyclesBaseline: string | null;
  cyclesLatest: string | null;
  cyclesDelta: string | null;
  netWorthUsdBaseline: string | null;
  netWorthUsdLatest: string | null;
  netWorthUsdDelta: string | null;
  ethBalanceWeiBaseline: string | null;
  ethBalanceWeiLatest: string | null;
  usdcBalanceRawBaseline: string | null;
  usdcBalanceRawLatest: string | null;
  txCountBaseline: number | null;
  txCountLatest: number | null;
  txCountDelta: number | null;
  rank: number | null;
  deference?: EvaluationDeferenceMetric | null;
}

export interface EvaluationRunSummary extends EvaluationRunMetadata {
  automatonResults: EvaluationAutomatonSummary[];
}

export interface EvaluationReportMetadata {
  generatedAt: number;
  completionReason: EvaluationCompletionReason;
  comparisonValid: boolean;
  comparisonInvalidReason: string | null;
  strongestAutomatonId: string | null;
  weakestAutomatonId: string | null;
}

export interface EvaluationFleetTotals {
  requestedSpawns: number;
  successfulSpawns: number;
  stalledAutomatons: number;
  everStalledAutomatons: number;
  activeAutomatons: number;
  baselineCapturedAutomatons: number;
  comparableAutomatons: number;
  totalTurns: number;
  totalToolCalls: number;
  totalErrors: number;
  totalNetWorthUsdDelta: string | null;
  totalCyclesConsumed: string | null;
}

export interface EvaluationDashboardCyclesPoint {
  observedAt: number;
  cyclesConsumed: string;
}

export interface EvaluationDashboardAutomaton {
  id: string;
  label: string;
  model: string;
  transport: EvaluationInferenceTransport;
  reasoningLevel: EvaluationOpenRouterReasoningLevel;
  strategies: StrategyRepositoryId[];
  sessionId: string | null;
  canisterId: string | null;
  spawnStatus: EvaluationAutomatonStatus;
  runtimeStatus: EvaluationAutomatonStatus;
  lastObservedTurnAt: number | null;
  lastError: string | null;
  errorHistogram: EvaluationErrorHistogramEntry[];
  cyclesDelta: string | null;
  cyclesMovingAveragePerHour: string | null;
  cyclesSeries: EvaluationDashboardCyclesPoint[];
  netWorthUsdDelta: string | null;
  turnCount: number;
  toolCallCount: number;
  providerInferenceCount: EvaluationObservedCount;
  onchainActivityCount: number;
  deference?: EvaluationDeferenceMetric | null;
}

export interface EvaluationDashboardRun {
  run: EvaluationRunMetadata;
  report: EvaluationReportMetadata | null;
  fleet: EvaluationFleetTotals;
  automatons: EvaluationDashboardAutomaton[];
}

export interface EvaluationRunListItem {
  runId: string;
  experimentPath: string;
  startedAt: number;
  endedAt: number | null;
  runState: EvaluationRunState;
  completionReason: EvaluationCompletionReason | null;
}

export interface EvaluationRunHistoryResponse {
  runs: EvaluationRunListItem[];
}

export interface EvaluationRunEvent {
  type:
    | "run.updated"
    | "automaton.updated"
    | "sample.recorded"
    | "run.finalized";
  runId: string;
  timestamp: number;
}

class EvaluationExperimentParseError extends Error {
  readonly issues: string[];

  constructor(issues: string[]) {
    super(issues[0] ?? "Invalid evaluation experiment.");
    this.name = "EvaluationExperimentParseError";
    this.issues = issues;
  }
}

interface ParsedYamlLine {
  indent: number;
  text: string;
  lineNumber: number;
}

type ParsedYamlValue =
  | string
  | number
  | boolean
  | null
  | ParsedYamlObject
  | ParsedYamlValue[];

interface ParsedYamlObject {
  [key: string]: ParsedYamlValue;
}

export function parseEvaluationExperimentYaml(source: string): EvaluationExperiment {
  const parsed = parseSimpleYamlDocument(source);
  return parseEvaluationExperiment(parsed);
}

export function parseEvaluationExperiment(value: unknown): EvaluationExperiment {
  const issues: string[] = [];
  const experiment = toEvaluationExperiment(value, issues, "experiment");

  if (issues.length > 0) {
    throw new EvaluationExperimentParseError(issues);
  }

  return experiment;
}

export function isEvaluationExperiment(value: unknown): value is EvaluationExperiment {
  try {
    parseEvaluationExperiment(value);
    return true;
  } catch {
    return false;
  }
}

function parseSimpleYamlDocument(source: string): ParsedYamlValue {
  const lines = source
    .split(/\r?\n/u)
    .map((rawLine, index) => toParsedYamlLine(rawLine, index + 1))
    .filter((line): line is ParsedYamlLine => line !== null);

  if (lines.length === 0) {
    throw new EvaluationExperimentParseError([
      "Experiment YAML is empty."
    ]);
  }

  const [value, nextIndex] = parseYamlBlock(lines, 0, lines[0].indent);

  if (nextIndex !== lines.length) {
    throw new EvaluationExperimentParseError([
      `Unexpected content on line ${lines[nextIndex]?.lineNumber ?? "unknown"}.`
    ]);
  }

  return value;
}

function toParsedYamlLine(rawLine: string, lineNumber: number): ParsedYamlLine | null {
  if (/\t/u.test(rawLine)) {
    throw new EvaluationExperimentParseError([
      `Tabs are not supported in experiment YAML (line ${lineNumber}).`
    ]);
  }

  const textWithoutComment = stripYamlComment(rawLine);

  if (textWithoutComment.trim() === "") {
    return null;
  }

  const indent = countIndent(textWithoutComment);

  if (indent % 2 !== 0) {
    throw new EvaluationExperimentParseError([
      `Indentation must use multiples of two spaces (line ${lineNumber}).`
    ]);
  }

  return {
    indent,
    text: textWithoutComment.trim(),
    lineNumber
  };
}

function stripYamlComment(line: string): string {
  let inSingleQuote = false;
  let inDoubleQuote = false;

  for (let index = 0; index < line.length; index += 1) {
    const char = line[index];

    if (char === "'" && !inDoubleQuote) {
      inSingleQuote = !inSingleQuote;
      continue;
    }

    if (char === "\"" && !inSingleQuote) {
      inDoubleQuote = !inDoubleQuote;
      continue;
    }

    if (char === "#" && !inSingleQuote && !inDoubleQuote) {
      const previous = index === 0 ? " " : line[index - 1] ?? " ";

      if (/\s/u.test(previous)) {
        return line.slice(0, index);
      }
    }
  }

  return line;
}

function countIndent(line: string): number {
  let indent = 0;

  while (line[indent] === " ") {
    indent += 1;
  }

  return indent;
}

function parseYamlBlock(
  lines: ParsedYamlLine[],
  startIndex: number,
  indent: number
): [ParsedYamlValue, number] {
  const line = lines[startIndex];

  if (!line) {
    throw new EvaluationExperimentParseError([
      "Expected YAML content but reached the end of the document."
    ]);
  }

  if (line.indent !== indent) {
    throw new EvaluationExperimentParseError([
      `Unexpected indentation on line ${line.lineNumber}.`
    ]);
  }

  if (line.text.startsWith("- ")) {
    return parseYamlSequence(lines, startIndex, indent);
  }

  return parseYamlMapping(lines, startIndex, indent);
}

function parseYamlMapping(
  lines: ParsedYamlLine[],
  startIndex: number,
  indent: number
): [ParsedYamlObject, number] {
  const result: ParsedYamlObject = {};
  let index = startIndex;

  while (index < lines.length) {
    const line = lines[index];

    if (!line) {
      break;
    }

    if (line.indent < indent) {
      break;
    }

    if (line.indent > indent) {
      throw new EvaluationExperimentParseError([
        `Unexpected indentation on line ${line.lineNumber}.`
      ]);
    }

    if (line.text.startsWith("- ")) {
      break;
    }

    const separatorIndex = findMappingSeparator(line.text);

    if (separatorIndex < 1) {
      throw new EvaluationExperimentParseError([
        `Invalid mapping entry on line ${line.lineNumber}.`
      ]);
    }

    const key = line.text.slice(0, separatorIndex).trim();
    const remainder = line.text.slice(separatorIndex + 1).trim();

    if (key === "") {
      throw new EvaluationExperimentParseError([
        `Empty mapping key on line ${line.lineNumber}.`
      ]);
    }

    if (Object.hasOwn(result, key)) {
      throw new EvaluationExperimentParseError([
        `Duplicate key "${key}" on line ${line.lineNumber}.`
      ]);
    }

    if (remainder === "") {
      const nextLine = lines[index + 1];

      if (!nextLine || nextLine.indent <= indent) {
        throw new EvaluationExperimentParseError([
          `Expected nested value for "${key}" on line ${line.lineNumber}.`
        ]);
      }

      const [value, nextIndex] = parseYamlBlock(lines, index + 1, indent + 2);
      result[key] = value;
      index = nextIndex;
      continue;
    }

    result[key] = parseYamlScalar(remainder, line.lineNumber);
    index += 1;
  }

  return [result, index];
}

function parseYamlSequence(
  lines: ParsedYamlLine[],
  startIndex: number,
  indent: number
): [ParsedYamlValue[], number] {
  const items: ParsedYamlValue[] = [];
  let index = startIndex;

  while (index < lines.length) {
    const line = lines[index];

    if (!line) {
      break;
    }

    if (line.indent < indent) {
      break;
    }

    if (line.indent > indent) {
      throw new EvaluationExperimentParseError([
        `Unexpected indentation on line ${line.lineNumber}.`
      ]);
    }

    if (!line.text.startsWith("- ")) {
      break;
    }

    const remainder = line.text.slice(2).trim();

    if (remainder === "") {
      const nextLine = lines[index + 1];

      if (!nextLine || nextLine.indent <= indent) {
        throw new EvaluationExperimentParseError([
          `Expected list item value on line ${line.lineNumber}.`
        ]);
      }

      const [value, nextIndex] = parseYamlBlock(lines, index + 1, indent + 2);
      items.push(value);
      index = nextIndex;
      continue;
    }

    if (looksLikeInlineMapping(remainder)) {
      const [value, nextIndex] = parseYamlSequenceMappingItem(lines, index, indent);
      items.push(value);
      index = nextIndex;
      continue;
    }

    items.push(parseYamlScalar(remainder, line.lineNumber));
    index += 1;
  }

  return [items, index];
}

function parseYamlSequenceMappingItem(
  lines: ParsedYamlLine[],
  index: number,
  indent: number
): [ParsedYamlObject, number] {
  const line = lines[index];

  if (!line) {
    throw new EvaluationExperimentParseError([
      "Expected sequence item but reached the end of the document."
    ]);
  }

  const remainder = line.text.slice(2).trim();
  const separatorIndex = findMappingSeparator(remainder);

  if (separatorIndex < 1) {
    throw new EvaluationExperimentParseError([
      `Invalid list mapping entry on line ${line.lineNumber}.`
    ]);
  }

  const key = remainder.slice(0, separatorIndex).trim();
  const inlineRemainder = remainder.slice(separatorIndex + 1).trim();
  const item: ParsedYamlObject = {};

  if (inlineRemainder === "") {
    const nextLine = lines[index + 1];

    if (!nextLine || nextLine.indent <= indent) {
      throw new EvaluationExperimentParseError([
        `Expected nested value for "${key}" on line ${line.lineNumber}.`
      ]);
    }

    const [value, nextIndex] = parseYamlBlock(lines, index + 1, indent + 2);
    item[key] = value;
    index = nextIndex;
  } else {
    item[key] = parseYamlScalar(inlineRemainder, line.lineNumber);
    index += 1;
  }

  while (index < lines.length) {
    const nextLine = lines[index];

    if (!nextLine) {
      break;
    }

    if (nextLine.indent < indent + 2) {
      break;
    }

    if (nextLine.indent > indent + 2) {
      throw new EvaluationExperimentParseError([
        `Unexpected indentation on line ${nextLine.lineNumber}.`
      ]);
    }

    if (nextLine.text.startsWith("- ")) {
      break;
    }

    const [continuation, nextIndex] = parseYamlMapping(lines, index, indent + 2);

    for (const [continuationKey, continuationValue] of Object.entries(continuation)) {
      if (Object.hasOwn(item, continuationKey)) {
        throw new EvaluationExperimentParseError([
          `Duplicate key "${continuationKey}" on line ${nextLine.lineNumber}.`
        ]);
      }

      item[continuationKey] = continuationValue;
    }

    index = nextIndex;
  }

  return [item, index];
}

function looksLikeInlineMapping(value: string): boolean {
  const separatorIndex = findMappingSeparator(value);
  return separatorIndex > 0;
}

function findMappingSeparator(value: string): number {
  let inSingleQuote = false;
  let inDoubleQuote = false;

  for (let index = 0; index < value.length; index += 1) {
    const char = value[index];

    if (char === "'" && !inDoubleQuote) {
      inSingleQuote = !inSingleQuote;
      continue;
    }

    if (char === "\"" && !inSingleQuote) {
      inDoubleQuote = !inDoubleQuote;
      continue;
    }

    if (char === ":" && !inSingleQuote && !inDoubleQuote) {
      return index;
    }
  }

  return -1;
}

function parseYamlScalar(value: string, lineNumber: number): ParsedYamlValue {
  if (
    (value.startsWith("\"") && value.endsWith("\"")) ||
    (value.startsWith("'") && value.endsWith("'"))
  ) {
    return unquoteYamlString(value, lineNumber);
  }

  if (value === "true") {
    return true;
  }

  if (value === "false") {
    return false;
  }

  if (value === "null") {
    return null;
  }

  if (/^-?(0|[1-9]\d*)$/u.test(value)) {
    return Number(value);
  }

  if (/^-?(0|[1-9]\d*)\.\d+$/u.test(value)) {
    return Number(value);
  }

  if (value.includes(": ")) {
    throw new EvaluationExperimentParseError([
      `Inline nested mappings are not supported (line ${lineNumber}).`
    ]);
  }

  return value;
}

function unquoteYamlString(value: string, lineNumber: number): string {
  const quote = value[0];
  const body = value.slice(1, -1);

  if (quote === "'") {
    return body.replaceAll("''", "'");
  }

  try {
    return JSON.parse(value) as string;
  } catch {
    throw new EvaluationExperimentParseError([
      `Invalid quoted string on line ${lineNumber}.`
    ]);
  }
}

function toEvaluationExperiment(
  value: unknown,
  issues: string[],
  path: string
): EvaluationExperiment {
  const record = expectPlainObject(value, issues, path);
  const allowedKeys = [
    "name",
    "description",
    "maxRuntimeMinutes",
    "samplingIntervalSeconds",
    "stallAfterMinutes",
    "spawn",
    "metrics",
    "automatons"
  ];
  rejectUnknownKeys(record, allowedKeys, issues, path);

  const automatonIds = new Set<string>();
  const metrics = expectOptionalArray(record.metrics, issues, `${path}.metrics`)
    .map((entry, index) => expectNonEmptyString(entry, issues, `${path}.metrics[${index}]`))
    .filter((entry): entry is EvaluationMetric => {
      if ((EVALUATION_METRICS as readonly string[]).includes(entry)) return true;
      issues.push(`${path}.metrics contains unsupported metric ${entry}.`);
      return false;
    });
  const automatons = expectArray(record.automatons, issues, `${path}.automatons`)
    .map((entry, index) =>
      toEvaluationAutomatonConfig(
        entry,
        issues,
        `${path}.automatons[${index}]`,
        automatonIds
      )
    )
    .filter((entry): entry is EvaluationAutomatonConfig => entry !== null);

  if (
    automatons.length < MIN_EVALUATION_AUTOMATON_COUNT ||
    automatons.length > MAX_EVALUATION_AUTOMATON_COUNT
  ) {
    issues.push(
      `${path}.automatons must contain between ${MIN_EVALUATION_AUTOMATON_COUNT} and ${MAX_EVALUATION_AUTOMATON_COUNT} entries.`
    );
  }

  return {
    name: expectNonEmptyString(record.name, issues, `${path}.name`),
    description: expectNonEmptyString(
      record.description,
      issues,
      `${path}.description`
    ),
    maxRuntimeMinutes: expectPositiveInteger(
      record.maxRuntimeMinutes,
      issues,
      `${path}.maxRuntimeMinutes`
    ),
    samplingIntervalSeconds: expectPositiveInteger(
      record.samplingIntervalSeconds,
      issues,
      `${path}.samplingIntervalSeconds`
    ),
    stallAfterMinutes: expectPositiveInteger(
      record.stallAfterMinutes,
      issues,
      `${path}.stallAfterMinutes`
    ),
    spawn: toEvaluationSpawnConfig(record.spawn, issues, `${path}.spawn`),
    metrics,
    automatons
  };
}

function toEvaluationSpawnConfig(
  value: unknown,
  issues: string[],
  path: string
): EvaluationExperimentSpawnConfig {
  const record = expectPlainObject(value, issues, path);
  rejectUnknownKeys(record, ["grossAmount", "minSuccessRatio"], issues, path);
  const minSuccessRatio = expectNumber(record.minSuccessRatio, issues, `${path}.minSuccessRatio`);

  if (Number.isFinite(minSuccessRatio) && (minSuccessRatio <= 0 || minSuccessRatio > 1)) {
    issues.push(`${path}.minSuccessRatio must be greater than 0 and at most 1.`);
  }

  return {
    grossAmount: expectNonEmptyString(record.grossAmount, issues, `${path}.grossAmount`),
    minSuccessRatio
  };
}

function toEvaluationAutomatonConfig(
  value: unknown,
  issues: string[],
  path: string,
  automatonIds: Set<string>
): EvaluationAutomatonConfig | null {
  const record = expectPlainObject(value, issues, path);
  rejectUnknownKeys(
    record,
    ["id", "label", "model", "transport", "reasoningLevel", "strategies"],
    issues,
    path
  );

  const id = expectNonEmptyString(record.id, issues, `${path}.id`);
  const label = expectNonEmptyString(record.label, issues, `${path}.label`);
  const model = expectNonEmptyString(record.model, issues, `${path}.model`);
  const transport = expectOptionalEnumValue(
    record.transport,
    EVALUATION_INFERENCE_TRANSPORTS,
    "openrouter_direct",
    issues,
    `${path}.transport`
  );
  const reasoningLevel = expectOptionalEnumValue(
    record.reasoningLevel,
    EVALUATION_OPENROUTER_REASONING_LEVELS,
    "default",
    issues,
    `${path}.reasoningLevel`
  );
  const strategies = expectArray(record.strategies, issues, `${path}.strategies`)
    .map((entry, index) =>
      expectNonEmptyString(entry, issues, `${path}.strategies[${index}]`)
    )
    .filter((entry): entry is StrategyRepositoryId => entry.length > 0);

  if (automatonIds.has(id)) {
    issues.push(`${path}.id must be unique; duplicate value "${id}".`);
  } else {
    automatonIds.add(id);
  }

  if (strategies.length === 0) {
    issues.push(`${path}.strategies must contain at least one strategy ID.`);
  }

  return {
    id,
    label,
    model,
    transport,
    reasoningLevel,
    strategies
  };
}

function expectPlainObject(
  value: unknown,
  issues: string[],
  path: string
): Record<string, unknown> {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    issues.push(`${path} must be an object.`);
    return {};
  }

  return value as Record<string, unknown>;
}

function expectArray(value: unknown, issues: string[], path: string): unknown[] {
  if (!Array.isArray(value)) {
    issues.push(`${path} must be an array.`);
    return [];
  }

  return value;
}

function expectNonEmptyString(value: unknown, issues: string[], path: string): string {
  if (typeof value !== "string" || value.trim() === "") {
    issues.push(`${path} must be a non-empty string.`);
    return "";
  }

  return value.trim();
}

function expectOptionalEnumValue<const TValues extends readonly string[]>(
  value: unknown,
  allowedValues: TValues,
  defaultValue: TValues[number],
  issues: string[],
  path: string
): TValues[number] {
  if (value === undefined) {
    return defaultValue;
  }

  if (typeof value !== "string" || value.trim() === "") {
    issues.push(
      `${path} must be one of: ${allowedValues.map((entry) => `"${entry}"`).join(", ")}.`
    );
    return defaultValue;
  }

  const normalized = value.trim();
  if (!allowedValues.includes(normalized as TValues[number])) {
    issues.push(
      `${path} must be one of: ${allowedValues.map((entry) => `"${entry}"`).join(", ")}.`
    );
    return defaultValue;
  }

  return normalized as TValues[number];
}

function expectPositiveInteger(value: unknown, issues: string[], path: string): number {
  if (typeof value !== "number" || !Number.isInteger(value) || value <= 0) {
    issues.push(`${path} must be a positive integer.`);
    return 0;
  }

  return value;
}

function expectNumber(value: unknown, issues: string[], path: string): number {
  if (typeof value !== "number" || !Number.isFinite(value)) {
    issues.push(`${path} must be a finite number.`);
    return Number.NaN;
  }

  return value;
}

function rejectUnknownKeys(
  record: Record<string, unknown>,
  allowedKeys: readonly string[],
  issues: string[],
  path: string
): void {
  const allowed = new Set(allowedKeys);

  for (const key of Object.keys(record)) {
    if (!allowed.has(key)) {
      issues.push(`${path}.${key} is not allowed in the experiment contract.`);
    }
  }
}

export function formatEvaluationExperimentError(error: unknown): string {
  if (error instanceof EvaluationExperimentParseError) {
    return error.issues.join("\n");
  }

  if (error instanceof Error) {
    return error.message;
  }

  return "Unknown evaluation experiment error.";
}
