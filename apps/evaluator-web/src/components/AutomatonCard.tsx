import type { EvaluationDashboardAutomaton } from "@ic-automaton/shared";

import {
  formatInferenceCount,
  formatMetric,
  formatTrillionCycles,
  formatTrillionCyclesPerHour,
  formatStrategies,
  formatTimestamp
} from "../lib/format";
import { CyclesSparkline } from "./CyclesSparkline";
import { ErrorHistogram } from "./ErrorHistogram";

interface AutomatonCardProps {
  automaton: EvaluationDashboardAutomaton;
}

export function AutomatonCard({ automaton }: AutomatonCardProps) {
  return (
    <article className="automaton-card">
      <header className="automaton-card__header">
        <div>
          <h3>{automaton.label}</h3>
          <p>{automaton.id}</p>
        </div>
        <div className="automaton-card__states">
          <span className="pill">{automaton.spawnStatus}</span>
          <span className="pill pill--muted">{automaton.runtimeStatus}</span>
        </div>
      </header>

      <dl className="automaton-card__details">
        <div>
          <dt>Model</dt>
          <dd>{automaton.model}</dd>
        </div>
        <div>
          <dt>Inference</dt>
          <dd>{`${automaton.transport} / ${automaton.reasoningLevel}`}</dd>
        </div>
        <div>
          <dt>Strategies</dt>
          <dd>{formatStrategies(automaton.strategies)}</dd>
        </div>
        <div>
          <dt>Canister</dt>
          <dd>{automaton.canisterId ?? "pending"}</dd>
        </div>
        <div>
          <dt>Last turn</dt>
          <dd>{formatTimestamp(automaton.lastObservedTurnAt)}</dd>
        </div>
        <div>
          <dt>Last error</dt>
          <dd>{automaton.lastError ?? "none"}</dd>
        </div>
        <div className="automaton-card__histogram">
          <dt>Error histogram</dt>
          <dd>
            <ErrorHistogram entries={automaton.errorHistogram} />
          </dd>
        </div>
        <div>
          <dt>Cycles delta</dt>
          <dd title={automaton.cyclesDelta ?? "n/a"}>
            {formatTrillionCycles(automaton.cyclesDelta)}
          </dd>
        </div>
        <div>
          <dt>Cycles / h MA</dt>
          <dd title={automaton.cyclesMovingAveragePerHour ?? "n/a"}>
            {formatTrillionCyclesPerHour(automaton.cyclesMovingAveragePerHour)}
          </dd>
        </div>
        <div className="automaton-card__trend">
          <dt>Cycles trend</dt>
          <dd>
            <CyclesSparkline points={automaton.cyclesSeries} />
          </dd>
        </div>
        <div>
          <dt>Net worth delta</dt>
          <dd>{formatMetric(automaton.netWorthUsdDelta)}</dd>
        </div>
        <div>
          <dt>Turns</dt>
          <dd>{automaton.turnCount}</dd>
        </div>
        <div>
          <dt>Tool calls</dt>
          <dd>{automaton.toolCallCount}</dd>
        </div>
        <div>
          <dt>Provider inferences</dt>
          <dd>{formatInferenceCount(automaton.providerInferenceCount)}</dd>
        </div>
        <div>
          <dt>Onchain activity</dt>
          <dd>{automaton.onchainActivityCount}</dd>
        </div>
      </dl>
    </article>
  );
}
