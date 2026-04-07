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

interface AutomatonTableProps {
  automatons: EvaluationDashboardAutomaton[];
}

export function AutomatonTable({ automatons }: AutomatonTableProps) {
  return (
    <section className="panel automaton-table-panel">
      <div className="panel__header">
        <h2>Automaton Fleet</h2>
        <p>All required run signals in one operator view.</p>
      </div>
      <div className="table-scroll">
        <table className="automaton-table">
          <thead>
            <tr>
              <th>Config</th>
              <th>Model</th>
              <th>Strategies</th>
              <th>Spawn</th>
              <th>Runtime</th>
              <th>Last turn</th>
              <th>Last error</th>
              <th>Error histogram</th>
              <th>Cycles delta</th>
              <th>Cycles / h MA</th>
              <th>Cycles trend</th>
              <th>Net worth delta</th>
              <th>Turns</th>
              <th>Tool calls</th>
              <th>Provider inferences</th>
              <th>Onchain activity</th>
            </tr>
          </thead>
          <tbody>
            {automatons.map((automaton) => (
              <tr key={automaton.id}>
                <td>
                  <strong>{automaton.label}</strong>
                  <div className="cell-subtle">{automaton.id}</div>
                  <div className="cell-subtle">{automaton.canisterId ?? "canister pending"}</div>
                </td>
                <td>{automaton.model}</td>
                <td>{formatStrategies(automaton.strategies)}</td>
                <td>{automaton.spawnStatus}</td>
                <td>{automaton.runtimeStatus}</td>
                <td>{formatTimestamp(automaton.lastObservedTurnAt)}</td>
                <td>{automaton.lastError ?? "none"}</td>
                <td>
                  <ErrorHistogram entries={automaton.errorHistogram} />
                </td>
                <td title={automaton.cyclesDelta ?? "n/a"}>
                  {formatTrillionCycles(automaton.cyclesDelta)}
                </td>
                <td title={automaton.cyclesMovingAveragePerHour ?? "n/a"}>
                  {formatTrillionCyclesPerHour(automaton.cyclesMovingAveragePerHour)}
                </td>
                <td>
                  <CyclesSparkline points={automaton.cyclesSeries} />
                </td>
                <td>{formatMetric(automaton.netWorthUsdDelta)}</td>
                <td>{automaton.turnCount}</td>
                <td>{automaton.toolCallCount}</td>
                <td>{formatInferenceCount(automaton.providerInferenceCount)}</td>
                <td>{automaton.onchainActivityCount}</td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </section>
  );
}
