export const EVALUATION_RUN_STATES = [
    "booting",
    "validating",
    "spawning",
    "running",
    "stopping",
    "completed",
    "aborted",
    "failed"
];
export const EVALUATION_AUTOMATON_STATUSES = [
    "pending_spawn",
    "spawning",
    "active",
    "stalled",
    "spawn_failed",
    "completed"
];
export const EVALUATION_COMPLETION_REASONS = [
    "completed",
    "timed_out",
    "stopped_manually",
    "aborted",
    "failed"
];
export const EVALUATION_PROVIDER_INFERENCE_UNAVAILABLE = "unavailable";
export const MIN_EVALUATION_AUTOMATON_COUNT = 1;
export const MAX_EVALUATION_AUTOMATON_COUNT = 10;
class EvaluationExperimentParseError extends Error {
    issues;
    constructor(issues) {
        super(issues[0] ?? "Invalid evaluation experiment.");
        this.name = "EvaluationExperimentParseError";
        this.issues = issues;
    }
}
export function parseEvaluationExperimentYaml(source) {
    const parsed = parseSimpleYamlDocument(source);
    return parseEvaluationExperiment(parsed);
}
export function parseEvaluationExperiment(value) {
    const issues = [];
    const experiment = toEvaluationExperiment(value, issues, "experiment");
    if (issues.length > 0) {
        throw new EvaluationExperimentParseError(issues);
    }
    return experiment;
}
export function isEvaluationExperiment(value) {
    try {
        parseEvaluationExperiment(value);
        return true;
    }
    catch {
        return false;
    }
}
function parseSimpleYamlDocument(source) {
    const lines = source
        .split(/\r?\n/u)
        .map((rawLine, index) => toParsedYamlLine(rawLine, index + 1))
        .filter((line) => line !== null);
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
function toParsedYamlLine(rawLine, lineNumber) {
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
function stripYamlComment(line) {
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
function countIndent(line) {
    let indent = 0;
    while (line[indent] === " ") {
        indent += 1;
    }
    return indent;
}
function parseYamlBlock(lines, startIndex, indent) {
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
function parseYamlMapping(lines, startIndex, indent) {
    const result = {};
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
function parseYamlSequence(lines, startIndex, indent) {
    const items = [];
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
function parseYamlSequenceMappingItem(lines, index, indent) {
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
    const item = {};
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
    }
    else {
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
function looksLikeInlineMapping(value) {
    const separatorIndex = findMappingSeparator(value);
    return separatorIndex > 0;
}
function findMappingSeparator(value) {
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
function parseYamlScalar(value, lineNumber) {
    if ((value.startsWith("\"") && value.endsWith("\"")) ||
        (value.startsWith("'") && value.endsWith("'"))) {
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
function unquoteYamlString(value, lineNumber) {
    const quote = value[0];
    const body = value.slice(1, -1);
    if (quote === "'") {
        return body.replaceAll("''", "'");
    }
    try {
        return JSON.parse(value);
    }
    catch {
        throw new EvaluationExperimentParseError([
            `Invalid quoted string on line ${lineNumber}.`
        ]);
    }
}
function toEvaluationExperiment(value, issues, path) {
    const record = expectPlainObject(value, issues, path);
    const allowedKeys = [
        "name",
        "description",
        "maxRuntimeMinutes",
        "samplingIntervalSeconds",
        "stallAfterMinutes",
        "spawn",
        "automatons"
    ];
    rejectUnknownKeys(record, allowedKeys, issues, path);
    const automatonIds = new Set();
    const automatons = expectArray(record.automatons, issues, `${path}.automatons`)
        .map((entry, index) => toEvaluationAutomatonConfig(entry, issues, `${path}.automatons[${index}]`, automatonIds))
        .filter((entry) => entry !== null);
    if (automatons.length < MIN_EVALUATION_AUTOMATON_COUNT ||
        automatons.length > MAX_EVALUATION_AUTOMATON_COUNT) {
        issues.push(`${path}.automatons must contain between ${MIN_EVALUATION_AUTOMATON_COUNT} and ${MAX_EVALUATION_AUTOMATON_COUNT} entries.`);
    }
    return {
        name: expectNonEmptyString(record.name, issues, `${path}.name`),
        description: expectNonEmptyString(record.description, issues, `${path}.description`),
        maxRuntimeMinutes: expectPositiveInteger(record.maxRuntimeMinutes, issues, `${path}.maxRuntimeMinutes`),
        samplingIntervalSeconds: expectPositiveInteger(record.samplingIntervalSeconds, issues, `${path}.samplingIntervalSeconds`),
        stallAfterMinutes: expectPositiveInteger(record.stallAfterMinutes, issues, `${path}.stallAfterMinutes`),
        spawn: toEvaluationSpawnConfig(record.spawn, issues, `${path}.spawn`),
        automatons
    };
}
function toEvaluationSpawnConfig(value, issues, path) {
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
function toEvaluationAutomatonConfig(value, issues, path, automatonIds) {
    const record = expectPlainObject(value, issues, path);
    rejectUnknownKeys(record, ["id", "label", "model", "strategies"], issues, path);
    const id = expectNonEmptyString(record.id, issues, `${path}.id`);
    const label = expectNonEmptyString(record.label, issues, `${path}.label`);
    const model = expectNonEmptyString(record.model, issues, `${path}.model`);
    const strategies = expectArray(record.strategies, issues, `${path}.strategies`)
        .map((entry, index) => expectNonEmptyString(entry, issues, `${path}.strategies[${index}]`))
        .filter((entry) => entry.length > 0);
    if (automatonIds.has(id)) {
        issues.push(`${path}.id must be unique; duplicate value "${id}".`);
    }
    else {
        automatonIds.add(id);
    }
    if (strategies.length === 0) {
        issues.push(`${path}.strategies must contain at least one strategy ID.`);
    }
    return {
        id,
        label,
        model,
        strategies
    };
}
function expectPlainObject(value, issues, path) {
    if (!value || typeof value !== "object" || Array.isArray(value)) {
        issues.push(`${path} must be an object.`);
        return {};
    }
    return value;
}
function expectArray(value, issues, path) {
    if (!Array.isArray(value)) {
        issues.push(`${path} must be an array.`);
        return [];
    }
    return value;
}
function expectNonEmptyString(value, issues, path) {
    if (typeof value !== "string" || value.trim() === "") {
        issues.push(`${path} must be a non-empty string.`);
        return "";
    }
    return value.trim();
}
function expectPositiveInteger(value, issues, path) {
    if (typeof value !== "number" || !Number.isInteger(value) || value <= 0) {
        issues.push(`${path} must be a positive integer.`);
        return 0;
    }
    return value;
}
function expectNumber(value, issues, path) {
    if (typeof value !== "number" || !Number.isFinite(value)) {
        issues.push(`${path} must be a finite number.`);
        return Number.NaN;
    }
    return value;
}
function rejectUnknownKeys(record, allowedKeys, issues, path) {
    const allowed = new Set(allowedKeys);
    for (const key of Object.keys(record)) {
        if (!allowed.has(key)) {
            issues.push(`${path}.${key} is not allowed in the experiment contract.`);
        }
    }
}
export function formatEvaluationExperimentError(error) {
    if (error instanceof EvaluationExperimentParseError) {
        return error.issues.join("\n");
    }
    if (error instanceof Error) {
        return error.message;
    }
    return "Unknown evaluation experiment error.";
}
