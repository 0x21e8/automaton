import type { EvaluationDashboardRun } from "@ic-automaton/shared";
import { useEffect, useState } from "react";

import { fetchCurrentRun, stopCurrentRun } from "./api/evaluator";
import {
  subscribeToEvaluatorEvents,
  type EvaluatorRealtimeEvent
} from "./api/ws";
import { AutomatonCard } from "./components/AutomatonCard";
import { AutomatonTable } from "./components/AutomatonTable";
import { FleetSummary } from "./components/FleetSummary";
import { RecentEvents, type DisplayEvent } from "./components/RecentEvents";
import { RunHeader } from "./components/RunHeader";
import { StopRunButton } from "./components/StopRunButton";

interface EvaluatorClient {
  fetchRun(signal?: AbortSignal): Promise<EvaluationDashboardRun | null>;
  stopRun(signal?: AbortSignal): Promise<{
    ok: boolean;
    accepted: boolean;
    run: EvaluationDashboardRun["run"] | null;
  }>;
  subscribe(handlers: {
    onEvent?: (event: EvaluatorRealtimeEvent) => void;
    onError?: (error: Error) => void;
    onOpen?: () => void;
  }): () => void;
}

interface AppProps {
  client?: EvaluatorClient;
  initialDashboard?: EvaluationDashboardRun | null;
  initialEvents?: DisplayEvent[];
}

const defaultClient: EvaluatorClient = {
  fetchRun: fetchCurrentRun,
  stopRun: stopCurrentRun,
  subscribe: subscribeToEvaluatorEvents
};

function describeEvent(event: EvaluatorRealtimeEvent) {
  const payload =
    typeof event.payload === "object" && event.payload !== null
      ? (event.payload as Record<string, unknown>)
      : null;
  const automatonId =
    payload !== null && typeof payload.automatonId === "string"
      ? payload.automatonId
      : null;
  const runState =
    payload !== null && typeof payload.runState === "string"
      ? payload.runState
      : null;
  const completionReason =
    payload !== null && typeof payload.completionReason === "string"
      ? payload.completionReason
      : null;
  const spawnStatus =
    payload !== null && typeof payload.spawnStatus === "string"
      ? payload.spawnStatus
      : null;
  const runtimeStatus =
    payload !== null && typeof payload.runtimeStatus === "string"
      ? payload.runtimeStatus
      : null;
  const lastError =
    payload !== null && typeof payload.lastError === "string" && payload.lastError.trim() !== ""
      ? payload.lastError.trim()
      : null;
  const baseline = payload !== null && payload.baseline === true;

  switch (event.type) {
    case "run.updated":
      return `Run state changed to ${runState ?? "updated"}`;
    case "automaton.updated":
      return lastError === null
        ? `${automatonId ?? "automaton"} -> spawn ${spawnStatus ?? "?"}, runtime ${runtimeStatus ?? "?"}`
        : `${automatonId ?? "automaton"} -> spawn ${spawnStatus ?? "?"}, runtime ${runtimeStatus ?? "?"}, error: ${lastError}`;
    case "sample.recorded":
      return baseline
        ? `Baseline captured for ${automatonId ?? "automaton"}`
        : `Sample recorded for ${automatonId ?? "automaton"}`;
    case "run.finalized":
      return `Run finalized as ${completionReason ?? "completed"}`;
    default:
      return event.type;
  }
}

function toDisplayEvent(event: EvaluatorRealtimeEvent): DisplayEvent {
  return {
    ...event,
    id: `${event.timestamp}-${event.type}-${Math.random().toString(36).slice(2, 8)}`,
    summary: describeEvent(event)
  };
}

function isTerminalRunState(runState: string) {
  return runState === "completed" || runState === "aborted" || runState === "failed";
}

export default function App({
  client = defaultClient,
  initialDashboard = null,
  initialEvents = []
}: AppProps) {
  const [dashboard, setDashboard] = useState<EvaluationDashboardRun | null>(initialDashboard);
  const [events, setEvents] = useState<DisplayEvent[]>(initialEvents);
  const [error, setError] = useState<string | null>(null);
  const [isLoading, setIsLoading] = useState(initialDashboard === null);
  const [isStopping, setIsStopping] = useState(false);
  const [realtimeStatus, setRealtimeStatus] = useState<"connecting" | "live" | "offline">(
    "connecting"
  );
  const [reloadTick, setReloadTick] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    let active = true;

    const load = async () => {
      if (dashboard === null) {
        setIsLoading(true);
      }

      try {
        const nextDashboard = await client.fetchRun(controller.signal);

        if (!active) {
          return;
        }

        setDashboard(nextDashboard);
        setError(null);
      } catch (loadError) {
        if (!active || controller.signal.aborted) {
          return;
        }

        setError(loadError instanceof Error ? loadError.message : String(loadError));
      } finally {
        if (active) {
          setIsLoading(false);
        }
      }
    };

    void load();

    return () => {
      active = false;
      controller.abort();
    };
  }, [client, reloadTick]);

  useEffect(() => {
    setRealtimeStatus("connecting");

    return client.subscribe({
      onOpen: () => {
        setRealtimeStatus("live");
      },
      onEvent: (event) => {
        setEvents((current) => [toDisplayEvent(event), ...current].slice(0, 25));
        setReloadTick((current) => current + 1);
      },
      onError: (subscriptionError) => {
        setRealtimeStatus("offline");
        setError(subscriptionError.message);
      }
    });
  }, [client]);

  const automatons = dashboard?.automatons ?? [];
  const stopDisabled =
    dashboard === null ||
    isTerminalRunState(dashboard.run.runState) ||
    dashboard.run.runState === "stopping";
  const eventCountLabel = `${events.length} recent event${events.length === 1 ? "" : "s"}`;

  return (
    <div className="app-shell">
      <div className="app-shell__chrome" />
      <main className="app-layout">
        <RunHeader
          dashboard={dashboard}
          error={error}
          isLoading={isLoading}
          realtimeStatus={realtimeStatus}
        />

        <section className="control-strip">
          <div>
            <p className="eyebrow">Operator Controls</p>
            <h2>Active run control plane</h2>
            <p className="control-strip__copy">
              REST hydrates the dashboard, websocket events trigger refreshes, and stop requests
              finalize artifacts through the evaluator backend.
            </p>
          </div>
          <div className="control-strip__actions">
            <span className="control-strip__events">{eventCountLabel}</span>
            <StopRunButton
              disabled={stopDisabled}
              isStopping={isStopping}
              onStop={() => {
                const controller = new AbortController();
                setIsStopping(true);
                setError(null);

                void client
                  .stopRun(controller.signal)
                  .then(() => {
                    setReloadTick((current) => current + 1);
                  })
                  .catch((stopError) => {
                    setError(
                      stopError instanceof Error ? stopError.message : String(stopError)
                    );
                  })
                  .finally(() => {
                    setIsStopping(false);
                  });
              }}
            />
          </div>
        </section>

        <FleetSummary fleet={dashboard?.fleet ?? null} />

        <div className="mobile-automaton-grid">
          {automatons.length === 0 ? (
            <div className="panel empty-state">No automaton rows are available yet.</div>
          ) : (
            automatons.map((automaton) => (
              <AutomatonCard automaton={automaton} key={automaton.id} />
            ))
          )}
        </div>

        <AutomatonTable automatons={automatons} />
        <RecentEvents events={events} />
      </main>
    </div>
  );
}
