import type { EvaluationErrorHistogramEntry } from "@ic-automaton/shared";

interface ErrorHistogramProps {
  entries: EvaluationErrorHistogramEntry[];
}

const MAX_ENTRIES = 4;

export function ErrorHistogram({ entries }: ErrorHistogramProps) {
  if (entries.length === 0) {
    return <span className="error-histogram__empty">none</span>;
  }

  const visibleEntries = entries.slice(0, MAX_ENTRIES);
  const maxCount = visibleEntries.reduce((current, entry) => Math.max(current, entry.count), 1);

  return (
    <div className="error-histogram" aria-label="Error histogram">
      {visibleEntries.map((entry) => {
        const width = Math.max(18, Math.round((entry.count / maxCount) * 100));

        return (
          <div className="error-histogram__row" key={`${entry.source}:${entry.message}`}>
            <div className="error-histogram__meta">
              <span className="error-histogram__source">{entry.source}</span>
              <span className="error-histogram__count">x{entry.count}</span>
            </div>
            <div className="error-histogram__bar-track">
              <div
                className="error-histogram__bar"
                style={{ width: `${width}%` }}
                title={`${entry.source} x${entry.count}: ${entry.message}`}
              />
            </div>
            <div className="error-histogram__message">{entry.message}</div>
          </div>
        );
      })}
      {entries.length > MAX_ENTRIES ? (
        <div className="error-histogram__more">+{entries.length - MAX_ENTRIES} more</div>
      ) : null}
    </div>
  );
}
