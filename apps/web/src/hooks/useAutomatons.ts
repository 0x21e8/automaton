import type { AutomatonListResponse } from "@ic-automaton/shared";
import { useEffect, useState } from "react";

import { fetchAutomatons } from "../api/indexer";
import { subscribeToRealtimeEvents } from "../api/ws";
import { getErrorMessage } from "../lib/errors";

const emptyResponse: AutomatonListResponse = {
  automatons: [],
  total: 0,
  prices: {
    ethUsd: null
  }
};

export function useAutomatons() {
  const [response, setResponse] = useState<AutomatonListResponse>(emptyResponse);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [refreshToken, setRefreshToken] = useState(0);

  useEffect(() => {
    const controller = new AbortController();

    setIsLoading(true);
    setError(null);

    void fetchAutomatons({
      signal: controller.signal
    })
      .then((nextResponse) => {
        setResponse(nextResponse);
      })
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) {
          return;
        }

        setResponse(emptyResponse);
        setError(getErrorMessage(nextError, "Unknown automaton feed error."));
      })
      .finally(() => {
        if (!controller.signal.aborted) {
          setIsLoading(false);
        }
      });

    return () => {
      controller.abort();
    };
  }, [refreshToken]);

  useEffect(() => {
    return subscribeToRealtimeEvents(
      {},
      {
        onEvent(event) {
          switch (event.type) {
            case "spawn":
            case "update":
            case "offline":
            case "spawn.session.completed":
              setRefreshToken((current) => current + 1);
              break;
            default:
              break;
          }
        }
      }
    );
  }, []);

  return {
    automatons: response.automatons,
    total: response.total,
    ethUsd: response.prices.ethUsd,
    isLoading,
    error,
    refresh() {
      setRefreshToken((current) => current + 1);
    }
  };
}
