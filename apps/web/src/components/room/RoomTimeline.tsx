import type { AutomatonSummary, RoomMessage } from "@ic-automaton/shared";

import {
  buildAutomatonNameLookup,
  resolveAutomatonLabel
} from "../../lib/room-messages";

const roomTimestampFormatter = new Intl.DateTimeFormat(undefined, {
  dateStyle: "medium",
  timeStyle: "short"
});

interface RoomTimelineProps {
  automatons: ReadonlyArray<AutomatonSummary>;
  error: string | null;
  isLoading: boolean;
  messages: ReadonlyArray<RoomMessage>;
}

function renderTimestamp(timestamp: number) {
  const date = new Date(timestamp);
  return {
    dateTime: date.toISOString(),
    label: roomTimestampFormatter.format(date)
  };
}

export function RoomTimeline({
  automatons,
  error,
  isLoading,
  messages
}: RoomTimelineProps) {
  const automatonNames = buildAutomatonNameLookup(automatons);

  return (
    <aside aria-label="Global room timeline" className="room-timeline">
      <div className="room-timeline-header">
        <p className="section-label">Global room</p>
        <h2 className="room-timeline-title">Room timeline</h2>
        <p className="room-timeline-copy">
          Indexed room history only. Message bodies render as inert plain text.
        </p>
      </div>

      {error !== null ? (
        <p className="room-timeline-notice is-error">Indexer unavailable: {error}</p>
      ) : null}
      {error === null && isLoading ? (
        <p className="room-timeline-notice">Loading indexed room history.</p>
      ) : null}
      {error === null && !isLoading && messages.length === 0 ? (
        <p className="room-timeline-notice">No indexed room messages yet.</p>
      ) : null}

      {messages.length > 0 ? (
        <ol className="room-message-list">
          {messages.map((message) => {
            const timestamp = renderTimestamp(message.createdAt);
            const authorLabel = resolveAutomatonLabel(
              message.authorCanisterId,
              automatonNames
            );

            return (
              <li className="room-message-item" key={message.messageId}>
                <article className="room-message-card">
                  <div className="room-message-meta">
                    <span className="room-message-badge">#{message.seq}</span>
                    <span className="room-message-badge">{message.contentType}</span>
                    <time className="room-message-badge" dateTime={timestamp.dateTime}>
                      {timestamp.label}
                    </time>
                  </div>

                  <p className="room-message-author">
                    From <strong>{authorLabel}</strong>
                  </p>

                  <p className="room-message-mentions">
                    {message.mentions.length === 0
                      ? "Broadcast"
                      : `Mentions: ${message.mentions
                          .map((canisterId) =>
                            resolveAutomatonLabel(canisterId, automatonNames)
                          )
                          .join(", ")}`}
                  </p>

                  <p className="room-message-body">{message.body}</p>
                </article>
              </li>
            );
          })}
        </ol>
      ) : null}
    </aside>
  );
}
