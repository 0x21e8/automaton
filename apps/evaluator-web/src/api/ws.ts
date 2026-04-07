import type { EvaluationRunEvent } from "@ic-automaton/shared";

import { buildEvaluatorWebsocketUrl } from "./evaluator";

export interface EvaluatorRealtimeEvent extends EvaluationRunEvent {
  payload?: unknown;
}

export interface EvaluatorRealtimeHandlers {
  onEvent?: (event: EvaluatorRealtimeEvent) => void;
  onError?: (error: Error) => void;
  onOpen?: () => void;
}

export function subscribeToEvaluatorEvents(
  handlers: EvaluatorRealtimeHandlers
): () => void {
  if (typeof window === "undefined" || typeof WebSocket === "undefined") {
    return () => {};
  }

  let disposed = false;
  let opened = false;
  const socket = new WebSocket(buildEvaluatorWebsocketUrl("/ws/events"));

  socket.addEventListener("open", () => {
    opened = true;

    if (disposed) {
      socket.close();
      return;
    }

    handlers.onOpen?.();
  });

  socket.addEventListener("message", (event) => {
    if (disposed) {
      return;
    }

    try {
      const payload = JSON.parse(String(event.data)) as EvaluatorRealtimeEvent;
      handlers.onEvent?.(payload);
    } catch (error) {
      handlers.onError?.(
        error instanceof Error ? error : new Error("Failed to decode evaluator event.")
      );
    }
  });

  socket.addEventListener("error", () => {
    if (!disposed) {
      handlers.onError?.(new Error("Evaluator realtime connection error."));
    }
  });

  return () => {
    disposed = true;

    if (opened || socket.readyState === WebSocket.OPEN) {
      socket.close();
    }
  };
}
