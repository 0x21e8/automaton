export function formatTimestamp(timestamp: number | null) {
  if (timestamp === null) {
    return "n/a";
  }

  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "medium"
  }).format(timestamp);
}

export function formatCompactTimestamp(timestamp: number) {
  return new Intl.DateTimeFormat(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit"
  }).format(timestamp);
}

export function formatMetric(value: string | number | null) {
  if (value === null) {
    return "n/a";
  }

  return String(value);
}

function formatTrillionUnit(value: string | null, suffix = "") {
  if (value === null) {
    return "n/a";
  }

  const normalized = value.trim();
  if (!/^-?\d+$/u.test(normalized)) {
    return normalized;
  }

  const sign = normalized.startsWith("-") ? "-" : "";
  const absolute = BigInt(sign === "-" ? normalized.slice(1) : normalized);
  const scale = absolute >= 1_000_000_000_000n ? 3 : 6;
  const divisor = 1_000_000_000_000n;
  const whole = absolute / divisor;
  const fraction = ((absolute % divisor) * 10n ** BigInt(scale)) / divisor;
  const renderedFraction = fraction.toString().padStart(scale, "0").replace(/0+$/u, "");

  return `${sign}${whole.toString()}${renderedFraction === "" ? "" : `.${renderedFraction}`}T${suffix}`;
}

export function formatTrillionCycles(value: string | null) {
  return formatTrillionUnit(value);
}

export function formatTrillionCyclesPerHour(value: string | null) {
  return formatTrillionUnit(value, "/h");
}

export function formatInferenceCount(value: number | "unavailable") {
  return value === "unavailable" ? "unavailable" : String(value);
}

export function formatStrategies(strategies: string[]) {
  return strategies.length === 0 ? "none" : strategies.join(", ");
}

export function formatRunState(state: string) {
  return state.replaceAll("_", " ");
}
