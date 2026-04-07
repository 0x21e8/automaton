import type { EvaluationDashboardRun } from "@ic-automaton/shared";

const EVALUATOR_BASE_URL = import.meta.env.VITE_EVALUATOR_BASE_URL?.trim() ?? "";

interface ApiErrorShape {
  error?: string;
  message?: string;
}

export interface StopRunResponse {
  ok: boolean;
  accepted: boolean;
  run: EvaluationDashboardRun["run"] | null;
}

function buildEvaluatorUrl(path: string): string {
  if (EVALUATOR_BASE_URL === "") {
    return path;
  }

  return new URL(path, EVALUATOR_BASE_URL).toString();
}

async function readErrorMessage(response: Response) {
  try {
    const payload = (await response.json()) as ApiErrorShape;
    return payload.error ?? payload.message ?? `Request failed with ${response.status}.`;
  } catch {
    return `Request failed with ${response.status}.`;
  }
}

async function requestEvaluatorJson<T>(
  path: string,
  options: Omit<RequestInit, "body"> & {
    body?: unknown;
  } = {}
) {
  const headers = new Headers(options.headers);
  headers.set("accept", "application/json");

  let body: string | undefined;

  if (options.body !== undefined) {
    headers.set("content-type", "application/json");
    body = JSON.stringify(options.body);
  }

  const response = await fetch(buildEvaluatorUrl(path), {
    ...options,
    headers,
    body
  });

  if (response.status === 404 && path === "/api/run") {
    return null as T;
  }

  if (!response.ok) {
    throw new Error(await readErrorMessage(response));
  }

  return (await response.json()) as T;
}

export async function fetchCurrentRun(signal?: AbortSignal) {
  return requestEvaluatorJson<EvaluationDashboardRun | null>("/api/run", {
    signal
  });
}

export async function stopCurrentRun(signal?: AbortSignal) {
  return requestEvaluatorJson<StopRunResponse>("/api/run/stop", {
    method: "POST",
    signal
  });
}

export function buildEvaluatorWebsocketUrl(path: string): string {
  const baseUrl =
    EVALUATOR_BASE_URL !== ""
      ? new URL(EVALUATOR_BASE_URL)
      : typeof window !== "undefined"
        ? new URL(window.location.origin)
        : null;

  if (baseUrl === null) {
    return path;
  }

  const protocol = baseUrl.protocol === "https:" ? "wss:" : "ws:";
  return new URL(path, `${protocol}//${baseUrl.host}`).toString();
}
