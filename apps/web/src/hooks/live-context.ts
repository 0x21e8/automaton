import { fetchAutomatonContext, type AutomatonContext } from "../api/automaton";

export const LIVE_CONTEXT_FRESHNESS_MS = 30_000;

const liveContextCache = new Map<string, { context: AutomatonContext; fetchedAt: number }>();

export async function loadLiveAutomatonContext(
  canisterUrl: string,
  signal?: AbortSignal
): Promise<AutomatonContext> {
  const cached = liveContextCache.get(canisterUrl);
  if (cached !== undefined && Date.now() - cached.fetchedAt < LIVE_CONTEXT_FRESHNESS_MS) {
    return cached.context;
  }

  const context = await fetchAutomatonContext(canisterUrl, signal);
  if (!signal?.aborted) {
    liveContextCache.set(canisterUrl, { context, fetchedAt: Date.now() });
  }
  return context;
}

export function clearLiveAutomatonContextCache() {
  liveContextCache.clear();
}
