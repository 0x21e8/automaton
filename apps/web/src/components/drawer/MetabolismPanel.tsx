import type { AutomatonDetail, AutomatonMetabolism } from "@ic-automaton/shared";

export function formatRunway(seconds: number | null): string {
  if (seconds === null) return "observing";
  if (seconds <= 0) return "0d";
  const days = seconds / 86_400;
  return days < 1 ? `${Math.max(1, Math.floor(seconds / 3_600))}h` : `${days.toFixed(days < 10 ? 1 : 0)}d`;
}

function formatBurn(value: number | null): string {
  if (value === null) return "observing";
  return `${(value / 1_000_000_000_000).toFixed(3)}T/day`;
}

function formatAge(seconds: number): string {
  const days = Math.floor(seconds / 86_400);
  return days > 0 ? `${days}d` : `${Math.floor(seconds / 3_600)}h`;
}

function formatEarnings(raw: string): string {
  try {
    const value = BigInt(raw);
    const whole = value / 1_000_000n;
    const fraction = (value % 1_000_000n).toString().padStart(6, "0").slice(0, 2);
    return `${whole}.${fraction} USDC`;
  } catch {
    return "0.00 USDC";
  }
}

function sparklinePoints(metabolism: AutomatonMetabolism): string {
  const values = metabolism.history.map((point) => point.runwaySeconds).filter((value): value is number => value !== null);
  if (values.length < 2) return "";
  const max = Math.max(...values, 1);
  return values.map((value, index) => `${index / (values.length - 1) * 100},${30 - value / max * 28}`).join(" ");
}

export function MetabolismPanel({ automaton }: { automaton: AutomatonDetail }) {
  const metabolism = automaton.metabolism ?? {
    burnRateCyclesPerDay: automaton.financials.burnRatePerDay,
    runwaySeconds: null,
    lifetimeEarningsUsdcRaw: "0",
    ageSeconds: Math.max(0, Math.floor((Date.now() - automaton.createdAt) / 1_000)),
    state: automaton.tier === "out_of_cycles" ? "dead" as const : "healthy" as const,
    history: []
  };
  const controlStatus = automaton.controlStatus ?? { label: "unverified" as const, controllers: [], spawnerPresent: false, verifiedAt: null };
  const points = sparklinePoints(metabolism);
  const controlCopy = controlStatus.label === "upgradeable_by_factory"
    ? "Upgradeable by the factory"
    : controlStatus.label === "self_controlled"
      ? "Self-controlled; no factory upgrade path"
      : controlStatus.label === "controller_mismatch"
        ? "Controller mismatch — chain review required"
        : "Controller status unverified";
  return <section className={`metabolism-panel metabolism-${metabolism.state}`} aria-labelledby="metabolism-heading">
    <div className="panel-heading">
      <h3 id="metabolism-heading">Metabolism</h3>
      <span className="panel-note">Indexer facts · {metabolism.state}</span>
    </div>
    <div className="metabolism-facts">
      <div><span>Burn</span><strong>{formatBurn(metabolism.burnRateCyclesPerDay)}</strong></div>
      <div><span>Runway</span><strong>{formatRunway(metabolism.runwaySeconds)}</strong></div>
      <div><span>Lifetime earnings</span><strong>{formatEarnings(metabolism.lifetimeEarningsUsdcRaw)}</strong></div>
      <div><span>Age</span><strong>{formatAge(metabolism.ageSeconds)}</strong></div>
    </div>
    <div className="metabolism-sparkline" aria-label="Runway history">
      {points ? <svg role="img" viewBox="0 0 100 32" preserveAspectRatio="none"><polyline points={points} /></svg> : <span>History begins after two indexed samples.</span>}
    </div>
    <details className="control-status">
      <summary>{controlCopy}</summary>
      {controlStatus.controllers.length > 0
        ? <ul>{controlStatus.controllers.map((controller) => <li key={controller}>{controller}</li>)}</ul>
        : <p>No controller attestation is indexed yet.</p>}
      {controlStatus.spawnerPresent ? <p role="alert">Spawner address appears in the controller list.</p> : null}
    </details>
  </section>;
}
