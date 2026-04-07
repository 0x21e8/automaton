import type { EvaluatorRealtimeEvent } from "../api/ws";
import { formatCompactTimestamp } from "../lib/format";

export interface DisplayEvent extends EvaluatorRealtimeEvent {
  id: string;
  summary: string;
}

interface RecentEventsProps {
  events: DisplayEvent[];
}

export function RecentEvents({ events }: RecentEventsProps) {
  return (
    <section className="panel">
      <div className="panel__header">
        <h2>Recent Events</h2>
        <p>Latest run transitions, automaton updates, and sampling writes.</p>
      </div>

      {events.length === 0 ? (
        <div className="empty-state">Waiting for evaluator events.</div>
      ) : (
        <ol className="event-list">
          {events.map((event) => (
            <li className="event-list__item" key={event.id}>
              <div>
                <strong>{event.summary}</strong>
                <p>{event.type}</p>
              </div>
              <time dateTime={new Date(event.timestamp).toISOString()}>
                {formatCompactTimestamp(event.timestamp)}
              </time>
            </li>
          ))}
        </ol>
      )}
    </section>
  );
}
