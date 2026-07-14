export interface MetabolismSampleInput {
  capturedAt: number;
  liquidCycles: number;
  usdcBalanceRaw: string | null;
}

export interface CanonicalRunwayInput {
  burnRateCyclesPerDay: number | null;
  liquidCycles: number;
  usdcBalanceRaw: string | null;
  usdcDecimals?: number;
  usdPerTrillionCycles?: number;
}

/** Must match the runtime's conservative conversion estimate for legacy snapshots. */
export const DEFAULT_USD_PER_TRILLION_CYCLES = 1.35;

function parseRawBalance(value: string | null): bigint {
  if (value === null) return 0n;
  try {
    return value.startsWith("0x") ? BigInt(value) : BigInt(value);
  } catch {
    return 0n;
  }
}

/**
 * The canonical runway formula used by every indexer consumer:
 *
 *   days = (liquid cycles + USDC-convertible cycles) / observed cycles burned per day
 *
 * USDC is converted with the same conservative cycles/USD estimate used by the
 * runtime. ETH is deliberately excluded: it is gas reserve, not immediately
 * convertible by the cycle-topup pipeline. Unknown or zero burn yields null.
 */
export function computeCanonicalRunwaySeconds(input: CanonicalRunwayInput): number | null {
  const burn = input.burnRateCyclesPerDay;
  if (burn === null || !Number.isFinite(burn) || burn <= 0) return null;

  const decimals = input.usdcDecimals ?? 6;
  const reportedUsdPerTrillion = input.usdPerTrillionCycles;
  const usdPerTrillion = reportedUsdPerTrillion !== undefined &&
      Number.isFinite(reportedUsdPerTrillion) && reportedUsdPerTrillion > 0
    ? reportedUsdPerTrillion
    : DEFAULT_USD_PER_TRILLION_CYCLES;
  const raw = parseRawBalance(input.usdcBalanceRaw);
  const usdc = Number(raw) / 10 ** decimals;
  const convertibleCycles = usdc * 1_000_000_000_000 / usdPerTrillion;
  const available = Math.max(0, input.liquidCycles) + Math.max(0, convertibleCycles);
  return Math.max(0, Math.floor(available / burn * 86_400));
}

export function computeWindowedBurnRate(
  samples: readonly MetabolismSampleInput[]
): number | null {
  if (samples.length < 2) return null;
  const ordered = [...samples].sort((left, right) => left.capturedAt - right.capturedAt);
  let observedDurationMs = 0;
  let consumedCycles = 0;

  for (let index = 1; index < ordered.length; index += 1) {
    const previous = ordered[index - 1];
    const current = ordered[index];
    if (!previous || !current) continue;
    const elapsedMs = current.capturedAt - previous.capturedAt;
    if (elapsedMs <= 0) continue;

    // A balance increase is a top-up interval. Exclude both its duration and
    // delta so added cycles cannot erase consumption observed elsewhere.
    if (current.liquidCycles > previous.liquidCycles) continue;

    observedDurationMs += elapsedMs;
    consumedCycles += previous.liquidCycles - current.liquidCycles;
  }

  if (observedDurationMs <= 0) return null;
  return consumedCycles / (observedDurationMs / 86_400_000);
}

export function positiveUsdcInflowRaw(previous: string | null, current: string | null): string {
  const delta = parseRawBalance(current) - parseRawBalance(previous);
  return delta > 0n ? delta.toString() : "0";
}

export function deriveMetabolicState(options: {
  runwaySeconds: number | null;
  tier: string;
  cyclesBalance: number;
}): "healthy" | "hibernating" | "dying" | "dead" {
  if (options.cyclesBalance <= 0 || options.tier === "out_of_cycles") return "dead";
  if (options.tier === "critical" || (options.runwaySeconds !== null && options.runwaySeconds < 86_400)) return "dying";
  if (options.tier === "low" || (options.runwaySeconds !== null && options.runwaySeconds < 7 * 86_400)) return "hibernating";
  return "healthy";
}
