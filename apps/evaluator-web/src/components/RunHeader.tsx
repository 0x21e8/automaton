import type { EvaluationDashboardRun } from "@ic-automaton/shared";

import { formatRunState, formatTimestamp } from "../lib/format";

interface RunHeaderProps {
  dashboard: EvaluationDashboardRun | null;
  error: string | null;
  isLoading: boolean;
  realtimeStatus: "connecting" | "live" | "offline";
}

function buildPhaseCopy(dashboard: EvaluationDashboardRun | null) {
  if (dashboard === null) {
    return null;
  }

  switch (dashboard.run.runState) {
    case "booting":
      return "Booting a fresh playground now. Watch the eval terminal for bootstrap logs; this phase can take a few minutes.";
    case "validating":
      return "Playground booted. The evaluator is validating live strategy IDs before the first spawn.";
    case "spawning":
      return "Spawn sessions are in flight. Each automaton will capture its baseline immediately after reaching complete.";
    case "running":
      return "Sampling is active. Evidence and derived counters update every 15 seconds.";
    case "completed":
      return "The run is finalized and artifacts are written to the local evaluation output directory.";
    case "aborted":
      return "The run aborted before comparison was considered valid. Inspect the abort reason and report artifacts.";
    case "failed":
      return "The run failed during setup or execution. Inspect the abort reason and terminal logs.";
    default:
      return null;
  }
}

function normalizeHeaderError(error: string | null, dashboard: EvaluationDashboardRun | null) {
  if (error === null) {
    return null;
  }

  if (error === "Failed to fetch" && dashboard === null) {
    return "Evaluator backend unreachable. Confirm that eval:dev or eval:run is still running and that the printed evaluator URL is correct.";
  }

  return error;
}

function buildHeadline(dashboard: EvaluationDashboardRun | null) {
  if (dashboard === null) {
    return "No active evaluation run";
  }

  if (dashboard.report !== null) {
    return dashboard.report.comparisonValid
      ? `Run ${dashboard.report.completionReason}`
      : `Run ${dashboard.report.completionReason} and invalid for comparison`;
  }

  return `Run ${formatRunState(dashboard.run.runState)}`;
}

export function RunHeader({
  dashboard,
  error,
  isLoading,
  realtimeStatus
}: RunHeaderProps) {
  const report = dashboard?.report ?? null;
  const phaseCopy = buildPhaseCopy(dashboard);
  const normalizedError = normalizeHeaderError(error, dashboard);
  const statusTone =
    dashboard?.run.runState === "aborted" || dashboard?.run.runState === "failed"
      ? "critical"
      : dashboard?.run.runState === "completed"
        ? "complete"
        : "live";
  const comparisonInvalid = report !== null && report.comparisonValid === false;

  return (
    <header className="run-header">
      <div className="run-header__intro">
        <p className="eyebrow">Automaton Evaluation Console</p>
        <h1>{buildHeadline(dashboard)}</h1>
        <p className="run-header__summary">
          {dashboard === null
            ? "Start the evaluator backend with an experiment file to populate this console."
            : `${dashboard.run.experimentPath} | requested ${dashboard.run.requestedAutomatonCount} | successful ${dashboard.run.successfulSpawnCount}`}
        </p>
      </div>
      <div className="run-header__meta">
        <span className={`status-chip status-chip--${statusTone}`}>
          {dashboard === null ? "idle" : formatRunState(dashboard.run.runState)}
        </span>
        <span className={`status-chip status-chip--${realtimeStatus}`}>
          ws {realtimeStatus}
        </span>
      </div>

      <dl className="run-details">
        <div>
          <dt>Experiment hash</dt>
          <dd>{dashboard?.run.experimentHash ?? "n/a"}</dd>
        </div>
        <div>
          <dt>Started</dt>
          <dd>{dashboard === null ? "n/a" : formatTimestamp(dashboard.run.startedAt)}</dd>
        </div>
        <div>
          <dt>Ended</dt>
          <dd>{dashboard === null ? "n/a" : formatTimestamp(dashboard.run.endedAt)}</dd>
        </div>
        <div>
          <dt>Launchpad commit</dt>
          <dd>{dashboard?.run.launchpadCommit ?? "n/a"}</dd>
        </div>
        <div>
          <dt>Child commit</dt>
          <dd>{dashboard?.run.childCommit ?? "unavailable"}</dd>
        </div>
        <div>
          <dt>Abort reason</dt>
          <dd>{dashboard?.run.abortReason ?? "none"}</dd>
        </div>
      </dl>

      {comparisonInvalid ? (
        <div className="run-banner run-banner--critical">
          Spawn success fell below the comparison threshold. The final report is visible, but
          this run should not be compared against other commits.
        </div>
      ) : null}

      {report !== null ? (
        <div className="run-banner run-banner--muted">
          Strongest: {report.strongestAutomatonId ?? "n/a"} | Weakest:{" "}
          {report.weakestAutomatonId ?? "n/a"}
        </div>
      ) : null}

      {phaseCopy !== null ? <div className="run-banner run-banner--phase">{phaseCopy}</div> : null}
      {isLoading ? <div className="run-banner run-banner--muted">Loading run state.</div> : null}
      {normalizedError !== null ? (
        <div className="run-banner run-banner--critical">{normalizedError}</div>
      ) : null}
    </header>
  );
}
