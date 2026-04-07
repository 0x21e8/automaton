import type { EvaluationFleetTotals } from "@ic-automaton/shared";

import { formatMetric } from "../lib/format";

interface FleetSummaryProps {
  fleet: EvaluationFleetTotals | null;
}

const summaryItems = [
  ["Requested spawns", "requestedSpawns"],
  ["Successful spawns", "successfulSpawns"],
  ["Stalled", "stalledAutomatons"],
  ["Active", "activeAutomatons"],
  ["Total turns", "totalTurns"],
  ["Tool calls", "totalToolCalls"],
  ["Errors", "totalErrors"],
  ["Net worth delta", "totalNetWorthUsdDelta"],
  ["Cycles consumed", "totalCyclesConsumed"]
] as const;

export function FleetSummary({ fleet }: FleetSummaryProps) {
  return (
    <section className="panel">
      <div className="panel__header">
        <h2>Fleet Summary</h2>
        <p>Aggregate signals across the full experiment fleet.</p>
      </div>
      <div className="metric-grid">
        {summaryItems.map(([label, key]) => (
          <article className="metric-card" key={key}>
            <span className="metric-card__label">{label}</span>
            <strong className="metric-card__value">
              {fleet === null ? "n/a" : formatMetric(fleet[key])}
            </strong>
          </article>
        ))}
      </div>
    </section>
  );
}
