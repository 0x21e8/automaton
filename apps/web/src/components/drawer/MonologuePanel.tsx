import { useState } from "react";

import type { JournalEntry, MonologueEntry } from "@ic-automaton/shared";

function formatTime(timestamp: number): string {
  return new Intl.DateTimeFormat("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
    timeZone: "UTC"
  }).format(timestamp);
}

type ActivityFilter = "important" | "all";

type FeedItem =
  | {
      id: string;
      kind: "entry";
      entry: MonologueEntry;
    }
  | {
      id: string;
      kind: "group";
      entries: MonologueEntry[];
    };

interface MonologuePanelProps {
  entries: readonly MonologueEntry[];
  journalEntries?: readonly JournalEntry[];
  errorMessage: string | null;
  isLoading: boolean;
  selectedCanisterId: string | null;
}

function isNotFoundError(errorMessage: string | null): boolean {
  return errorMessage?.toLowerCase().includes("not found") ?? false;
}

function sortEntries(entries: readonly MonologueEntry[]) {
  return [...entries].sort((left, right) => {
    if (left.timestamp === right.timestamp) {
      return right.turnId.localeCompare(left.turnId);
    }

    return right.timestamp - left.timestamp;
  });
}

function buildFeedItems(entries: readonly MonologueEntry[], filter: ActivityFilter): FeedItem[] {
  const sortedEntries = sortEntries(entries);

  if (filter === "important") {
    return sortedEntries
      .filter((entry) => entry.importance !== "low")
      .map((entry) => ({
        id: entry.turnId,
        kind: "entry",
        entry
      }));
  }

  const items: FeedItem[] = [];

  for (let index = 0; index < sortedEntries.length; index += 1) {
    const entry = sortedEntries[index];

    if (entry.importance !== "low" || entry.type !== "thought") {
      items.push({
        id: entry.turnId,
        kind: "entry",
        entry
      });
      continue;
    }

    const groupEntries = [entry];
    let nextIndex = index + 1;

    while (nextIndex < sortedEntries.length) {
      const nextEntry = sortedEntries[nextIndex];

      if (nextEntry.importance !== "low" || nextEntry.type !== "thought") {
        break;
      }

      groupEntries.push(nextEntry);
      nextIndex += 1;
    }

    if (groupEntries.length === 1) {
      items.push({
        id: entry.turnId,
        kind: "entry",
        entry
      });
    } else {
      items.push({
        id: `group:${groupEntries[0]?.turnId ?? index}:${groupEntries.at(-1)?.turnId ?? index}`,
        kind: "group",
        entries: groupEntries
      });
    }

    index = nextIndex - 1;
  }

  return items;
}

function getCategoryLabel(entry: MonologueEntry) {
  switch (entry.category) {
    case "act":
      return "action";
    case "decide":
      return "decision";
    case "message":
      return "message";
    case "error":
      return "error";
    default:
      return "observe";
  }
}

function summarizeGroup(entries: readonly MonologueEntry[]) {
  const decisionCount = entries.filter((entry) => entry.category === "decide").length;

  if (decisionCount > 0) {
    return `${entries.length} low-signal observation updates hidden`;
  }

  return `${entries.length} quiet monitoring updates hidden`;
}

function formatGroupRange(entries: readonly MonologueEntry[]) {
  const firstEntry = entries[0];
  const lastEntry = entries.at(-1);

  if (!firstEntry || !lastEntry) {
    return "";
  }

  return `${formatTime(firstEntry.timestamp)}-${formatTime(lastEntry.timestamp)}`;
}

export function MonologuePanel({
  entries,
  journalEntries = [],
  errorMessage,
  isLoading,
  selectedCanisterId
}: MonologuePanelProps) {
  const [filter, setFilter] = useState<ActivityFilter>("important");
  const [showOperatorDebug, setShowOperatorDebug] = useState(false);
  const [expandedIds, setExpandedIds] = useState<Record<string, boolean>>({});
  const feedItems = buildFeedItems(entries, filter);
  const emptyCopy = isLoading
    ? "Loading indexed turns."
    : errorMessage !== null
      ? isNotFoundError(errorMessage)
        ? selectedCanisterId === null
          ? "No indexed activity is available for the selected automaton."
          : `No indexed activity is available for ${selectedCanisterId}.`
        : "Activity is unavailable until the detail request succeeds."
      : entries.length === 0
        ? "No indexed turns yet. Recent activity appears after the next successful poll."
        : "No important activity in the latest indexed turns.";

  function toggleExpanded(id: string) {
    setExpandedIds((current) => ({
      ...current,
      [id]: !current[id]
    }));
  }

  return (
    <section className="log-section" aria-labelledby="monologue-heading">
      <div className="panel-heading">
        <div>
          <h3 id="monologue-heading">{showOperatorDebug ? "Operator Debug" : "Public Journal"}</h3>
          <span className="panel-note">
            {showOperatorDebug ? "Runtime diagnostics from indexed turns" : "The being's own public voice"}
          </span>
        </div>
        <div className="activity-filter" role="tablist" aria-label="Journal view">
          <button
            className={`activity-filter-btn${!showOperatorDebug ? " is-active" : ""}`}
            onClick={() => setShowOperatorDebug(false)}
            type="button"
          >
            Journal
          </button>
          <button
            className={`activity-filter-btn${showOperatorDebug ? " is-active" : ""}`}
            onClick={() => setShowOperatorDebug(true)}
            type="button"
          >
            Operator debug
          </button>
          {showOperatorDebug ? <>
          <button
            className={`activity-filter-btn${filter === "important" ? " is-active" : ""}`}
            onClick={() => {
              setFilter("important");
            }}
            type="button"
          >
            Important
          </button>
          <button
            className={`activity-filter-btn${filter === "all" ? " is-active" : ""}`}
            onClick={() => {
              setFilter("all");
            }}
            type="button"
          >
            All
          </button>
          </> : null}
        </div>
      </div>

      <div className="log-feed">
        {!showOperatorDebug ? (
          journalEntries.length > 0 ? (
            [...journalEntries]
              .sort((left, right) => right.id - left.id)
              .map((entry) => (
                <article className="activity-card is-medium" key={entry.id}>
                  <div className="activity-topline">
                    <span className="log-time">{formatTime(entry.timestamp)}</span>
                    <span className="activity-kind is-message">
                      {entry.genesis ? "credo" : "journal"}
                    </span>
                  </div>
                  <p className="activity-detail">{entry.text}</p>
                  {entry.dealClaim ? (
                    <p className={`room-message-settlement is-${entry.settlement?.status ?? "unsettled"}`}>
                      {entry.settlement?.status === "settled"
                        ? `Settled on-chain · ${entry.settlement.amountRaw} ${entry.settlement.asset?.toUpperCase()} raw · ${entry.settlement.txHash}`
                        : `Unsettled journal claim · ${entry.dealClaim.txHash}`}
                    </p>
                  ) : null}
                </article>
              ))
          ) : (
            <p className="empty-copy">
              {isLoading ? "Loading the public journal." : "No public journal entries yet."}
            </p>
          )
        ) : feedItems.length > 0 ? (
          feedItems.map((item) => {
            const isExpanded = expandedIds[item.id] ?? false;

            if (item.kind === "group") {
              return (
                <article className="activity-card is-group" key={item.id}>
                  <div className="activity-topline">
                    <span className="log-time">{formatGroupRange(item.entries)}</span>
                    <span className="activity-kind is-observe">quiet</span>
                    <button
                      className="activity-toggle"
                      onClick={() => {
                        toggleExpanded(item.id);
                      }}
                      type="button"
                    >
                      {isExpanded ? "Hide" : "Show"}
                    </button>
                  </div>
                  <p className="activity-headline">{summarizeGroup(item.entries)}</p>
                  <div className="activity-meta">
                    <span>{item.entries.length} thought-only turns collapsed</span>
                  </div>
                  {isExpanded ? (
                    <div className="activity-detail">
                      <ul className="activity-group-list">
                        {item.entries.map((entry) => (
                          <li key={entry.turnId}>
                            <span>{formatTime(entry.timestamp)}</span>
                            <span>{entry.headline}</span>
                          </li>
                        ))}
                      </ul>
                    </div>
                  ) : null}
                </article>
              );
            }

            const { entry } = item;

            return (
              <article
                className={`activity-card is-${entry.importance}${entry.error ? " has-error" : ""}`}
                key={entry.turnId}
              >
                <div className="activity-topline">
                  <span className="log-time">{formatTime(entry.timestamp)}</span>
                  <span className={`activity-kind is-${entry.category}`}>
                    {getCategoryLabel(entry)}
                  </span>
                  <button
                    className="activity-toggle"
                    onClick={() => {
                      toggleExpanded(item.id);
                    }}
                    type="button"
                  >
                    {isExpanded ? "Hide" : "Show"}
                  </button>
                </div>
                <p className="activity-headline">{entry.headline}</p>
                <div className="activity-meta">
                  <span>{entry.agentState}</span>
                  {entry.toolCallCount > 0 ? (
                    <span>
                      {entry.toolCallCount} tool{entry.toolCallCount === 1 ? "" : "s"}
                    </span>
                  ) : null}
                  {entry.durationMs !== null ? <span>{entry.durationMs}ms</span> : null}
                  {entry.error !== null ? <span className="is-error">error</span> : null}
                </div>
                {isExpanded ? (
                  <div className="activity-detail">
                    <p>{entry.message}</p>
                    {entry.error !== null ? (
                      <p className="activity-error">Error: {entry.error}</p>
                    ) : null}
                  </div>
                ) : null}
              </article>
            );
          })
        ) : (
          <p className="empty-copy">{emptyCopy}</p>
        )}
      </div>
    </section>
  );
}
