import type { RoomMessage } from "@ic-automaton/shared";
import { useEffect, useState } from "react";

import { fetchRoomHistory } from "../api/indexer";
import { subscribeToRealtimeEvents } from "../api/ws";
import { getErrorMessage } from "../lib/errors";
import { mergeRoomMessages } from "../lib/room-messages";

export function useRoomTimeline() {
  const [messages, setMessages] = useState<RoomMessage[]>([]);
  const [isLoading, setIsLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const controller = new AbortController();

    setIsLoading(true);
    setError(null);

    void fetchRoomHistory(controller.signal)
      .then((page) => {
        setMessages((current) => mergeRoomMessages(current, page.messages));
      })
      .catch((nextError: unknown) => {
        if (controller.signal.aborted) {
          return;
        }

        setError(getErrorMessage(nextError, "Unknown room timeline error."));
      })
      .finally(() => {
        if (!controller.signal.aborted) {
          setIsLoading(false);
        }
      });

    return () => {
      controller.abort();
    };
  }, []);

  useEffect(() => {
    return subscribeToRealtimeEvents(
      {},
      {
        onEvent(event) {
          if (event.type !== "message") {
            return;
          }

          setError(null);
          setMessages((current) => mergeRoomMessages(current, [event.message]));
        }
      }
    );
  }, []);

  return {
    messages,
    isLoading,
    error
  };
}
