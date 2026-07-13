import type { AutomatonDetail } from "@ic-automaton/shared";
import { useEffect, useState } from "react";

import { fetchAutomatonDetail } from "../api/indexer";
import { subscribeToRealtimeEvents } from "../api/ws";
import { getErrorMessage } from "../lib/errors";

function mergeMonologueEntries(
  existingEntries: ReadonlyArray<AutomatonDetail["monologue"][number]>,
  nextEntry: AutomatonDetail["monologue"][number]
) {
  const byKey = new Map<string, AutomatonDetail["monologue"][number]>();

  for (const entry of existingEntries) {
    byKey.set(`${entry.timestamp}:${entry.turnId}`, entry);
  }

  byKey.set(`${nextEntry.timestamp}:${nextEntry.turnId}`, nextEntry);

  return [...byKey.values()].sort((left, right) => {
    if (left.timestamp === right.timestamp) {
      return right.turnId.localeCompare(left.turnId);
    }

    return right.timestamp - left.timestamp;
  });
}

function mergeJournalEntries(
  existingEntries: NonNullable<AutomatonDetail["journal"]>,
  nextEntry: NonNullable<AutomatonDetail["journal"]>[number]
) {
  const byId = new Map(existingEntries.map((entry) => [entry.id, entry]));
  byId.set(nextEntry.id, nextEntry);
  return [...byId.values()].sort((left, right) => right.id - left.id);
}

export function useAutomatonDetail(canisterId: string | null) {
  const [automaton, setAutomaton] = useState<AutomatonDetail | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshToken, setRefreshToken] = useState(0);

  useEffect(() => {
    if (canisterId === null) {
      setAutomaton(null);
      setError(null);
      setIsLoading(false);
      return;
    }

    const controller = new AbortController();

    setIsLoading(true);
    setError(null);

    void fetchAutomatonDetail(canisterId, controller.signal)
      .then((detail) => {
        setAutomaton(detail);
      })
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) {
          return;
        }

        setAutomaton(null);
        setError(getErrorMessage(nextError, "Unknown automaton detail error."));
      })
      .finally(() => {
        if (!controller.signal.aborted) {
          setIsLoading(false);
        }
      });

    return () => {
      controller.abort();
    };
  }, [canisterId, refreshToken]);

  useEffect(() => {
    if (canisterId === null) {
      return;
    }

    return subscribeToRealtimeEvents(
      {
        canisterId
      },
      {
        onEvent(event) {
          if (event.type === "monologue") {
            setAutomaton((current) => {
              if (current === null || current.canisterId !== event.canisterId) {
                return current;
              }

              return {
                ...current,
                monologue: mergeMonologueEntries(current.monologue, event.entry)
              };
            });
            return;
          }

          if (event.type === "journal") {
            setAutomaton((current) => {
              if (current === null || current.canisterId !== event.canisterId) return current;
              return {
                ...current,
                journal: mergeJournalEntries(current.journal ?? [], event.entry)
              };
            });
            return;
          }

          setRefreshToken((current) => current + 1);
        }
      }
    );
  }, [canisterId]);

  return {
    automaton,
    isLoading,
    error,
    refresh() {
      setRefreshToken((current) => current + 1);
    }
  };
}
