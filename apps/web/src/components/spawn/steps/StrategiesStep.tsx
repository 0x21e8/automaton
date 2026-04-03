import type { RepositoryStrategyRecord } from "@ic-automaton/shared";

interface StrategiesStepProps {
  chainLabel: string;
  errorMessage: string | null;
  isLoading: boolean;
  strategies: RepositoryStrategyRecord[];
  selectedIds: string[];
  onToggle: (id: string) => void;
}

export function StrategiesStep({
  chainLabel,
  errorMessage,
  isLoading,
  strategies,
  selectedIds,
  onToggle
}: StrategiesStepProps) {
  return (
    <section className="spawn-step">
      <p className="section-label">Step 2</p>
      <h3 className="spawn-step-title">Strategies</h3>
      <p className="spawn-step-copy">
        Choose concrete repository-backed templates for this {chainLabel} spawn.
        The wizard only shows active templates compatible with the selected chain.
      </p>

      {isLoading ? (
        <p className="spawn-step-copy">Loading repository strategies.</p>
      ) : null}

      {errorMessage !== null ? (
        <p className="spawn-session-error" role="alert">
          {errorMessage}
        </p>
      ) : null}

      {!isLoading && errorMessage === null && strategies.length === 0 ? (
        <p className="spawn-step-copy">
          No active repository strategies are currently available for {chainLabel}.
        </p>
      ) : null}

      <div className="spawn-checklist">
        {strategies.map((strategy) => {
          const checked = selectedIds.includes(strategy.strategyId);

          return (
            <button
              aria-pressed={checked}
              className={`spawn-check-item${checked ? " is-checked" : ""}`}
              key={strategy.strategyId}
              onClick={() => {
                onToggle(strategy.strategyId);
              }}
              type="button"
            >
              <span className="spawn-check-mark">{checked ? "×" : ""}</span>
              <span className="spawn-check-body">
                <span className="spawn-check-title">{strategy.name}</span>
                <span className="spawn-check-copy">{strategy.description}</span>
              </span>
              <span className="spawn-check-meta">
                {strategy.protocol} · {strategy.primitive}
              </span>
            </button>
          );
        })}
      </div>
    </section>
  );
}
