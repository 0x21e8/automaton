import type { ChronicleDay } from "@ic-automaton/shared";
import { useEffect, useState } from "react";
import { fetchChronicle } from "../api/indexer";
import { getErrorMessage } from "../lib/errors";

export function useChronicle() {
  const [days, setDays] = useState<ChronicleDay[]>([]);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    const controller = new AbortController();
    void fetchChronicle(controller.signal).then((feed) => setDays(feed.days)).catch((reason) => {
      if (!controller.signal.aborted) setError(getErrorMessage(reason, "Chronicle unavailable."));
    });
    return () => controller.abort();
  }, []);
  return { days, error };
}
