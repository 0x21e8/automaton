import type {
  CreateSpawnSessionRequest,
  SpawnAsset,
  SpawnQuote,
  SpawnSession,
  SpawnSessionDetail
} from "@ic-automaton/shared";
import { useEffect, useState } from "react";

import {
  createSpawnSession,
  fetchSpawnSessionDetail,
  refundSpawnSession,
  retrySpawnSession
} from "../api/spawn";
import { subscribeToRealtimeEvents } from "../api/ws";
import { getErrorMessage } from "../lib/errors";

const SPAWN_SESSION_POLL_INTERVAL_MS = 4_000;

function isPollingComplete(session: SpawnSession | null): boolean {
  if (session === null) {
    return true;
  }

  if (session.state === "complete") {
    return true;
  }

  return session.state === "expired" && session.paymentStatus === "refunded";
}

function formatAssetAmount(amount: string, asset: SpawnAsset): string {
  return `${amount} ${asset.toUpperCase()}`;
}

function titleCase(value: string): string {
  return value
    .split("_")
    .map((part) => part.slice(0, 1).toUpperCase() + part.slice(1))
    .join(" ");
}

export function formatSpawnSessionStateLabel(value: string): string {
  return titleCase(value);
}

export function describeSpawnSessionProgress(session: SpawnSession): string {
  switch (session.state) {
    case "awaiting_payment":
      return "Waiting for the quoted escrow payment to arrive before the factory can proceed.";
    case "payment_detected":
      return "Escrow payment was detected. The factory is preparing the birth pipeline.";
    case "spawning":
      return "The factory is creating the automaton canister and provisioning its founding state.";
    case "broadcasting_release":
      return "Canister install succeeded. The factory is broadcasting the escrow release transaction.";
    case "complete":
      return "Birth completed and the new automaton should now appear on the grid.";
    case "failed":
      return session.retryable
        ? "Birth failed but can still be retried before expiration."
        : "Birth failed and is waiting for expiration before any refund path opens.";
    case "expired":
      return session.refundable
        ? "Funds can now be reclaimed through refund. This session expired unresolved because the quote TTL may have elapsed or the playground may have reset."
        : "This session expired. The quote TTL may have elapsed or the playground may have reset before completion.";
    default:
      return "Birth status unavailable.";
  }
}

export function derivePaymentInstructions(
  session: SpawnSession | null,
  detail: SpawnSessionDetail | null,
  quote: SpawnQuote | null
) {
  if (session === null) {
    return null;
  }

  const paymentAddress =
    detail?.payment.paymentAddress ?? quote?.payment.paymentAddress ?? null;

  if (paymentAddress === null) {
    return null;
  }

  return {
    sessionId: detail?.payment.sessionId ?? quote?.payment.sessionId ?? session.sessionId,
    claimId: detail?.payment.claimId ?? quote?.payment.claimId ?? session.claimId,
    chain: detail?.payment.chain ?? quote?.payment.chain ?? session.chain,
    paymentAddress,
    grossAmount: detail?.payment.grossAmount ?? quote?.payment.grossAmount ?? session.grossAmount,
    asset: session.asset,
    quoteTermsHash: session.quoteTermsHash,
    expiresAt: session.expiresAt
  };
}

export function useSpawnSession() {
  const [sessionId, setSessionId] = useState<string | null>(null);
  const [quote, setQuote] = useState<SpawnQuote | null>(null);
  const [detail, setDetail] = useState<SpawnSessionDetail | null>(null);
  const [provisionalSession, setProvisionalSession] = useState<SpawnSession | null>(
    null
  );
  const [error, setError] = useState<string | null>(null);
  const [isCreating, setIsCreating] = useState(false);
  const [isRefreshing, setIsRefreshing] = useState(false);
  const [isMutating, setIsMutating] = useState(false);
  const [refreshToken, setRefreshToken] = useState(0);

  const session = detail?.session ?? provisionalSession;

  async function refresh(targetSessionId = sessionId) {
    if (targetSessionId === null) {
      return null;
    }

    setIsRefreshing(true);

    try {
      const nextDetail = await fetchSpawnSessionDetail(targetSessionId);
      setDetail(nextDetail);
      setProvisionalSession(nextDetail.session);
      setError(null);
      return nextDetail;
    } catch (nextError: unknown) {
      setError(getErrorMessage(nextError, "Unknown birth session error."));
      return null;
    } finally {
      setIsRefreshing(false);
    }
  }

  useEffect(() => {
    if (sessionId === null) {
      return;
    }

    void refresh(sessionId);
  }, [refreshToken, sessionId]);

  useEffect(() => {
    if (sessionId === null || isPollingComplete(session)) {
      return;
    }

    const timer = window.setInterval(() => {
      setRefreshToken((current) => current + 1);
    }, SPAWN_SESSION_POLL_INTERVAL_MS);

    return () => {
      window.clearInterval(timer);
    };
  }, [session, sessionId]);

  useEffect(() => {
    if (sessionId === null) {
      return;
    }

    return subscribeToRealtimeEvents(
      {
        sessionId
      },
      {
        onEvent(event) {
          switch (event.type) {
            case "spawn.session.updated":
            case "spawn.session.completed":
            case "spawn.session.failed":
            case "spawn.session.expired":
              setDetail((current) => {
                const payment = current?.payment ?? quote?.payment;
                if (!payment) {
                  return current;
                }

                return {
                  session: event.session,
                  payment,
                  audit: event.audit,
                  registryRecord: current?.registryRecord ?? null
                };
              });
              setProvisionalSession(event.session);

              if (event.type !== "spawn.session.updated") {
                setRefreshToken((current) => current + 1);
              }
              break;
            default:
              break;
          }
        }
      }
    );
  }, [quote, sessionId]);

  return {
    sessionId,
    session,
    detail,
    quote,
    error,
    isCreating,
    isRefreshing,
    isMutating,
    paymentInstructions: derivePaymentInstructions(session, detail, quote),
    async create(request: CreateSpawnSessionRequest) {
      setIsCreating(true);
      setError(null);

      try {
        const response = await createSpawnSession(request);
        setSessionId(response.session.sessionId);
        setQuote(response.quote);
        setDetail(null);
        setProvisionalSession(response.session);
        setRefreshToken((current) => current + 1);
        return response;
      } catch (nextError: unknown) {
        setError(getErrorMessage(nextError, "Unknown birth session error."));
        return null;
      } finally {
        setIsCreating(false);
      }
    },
    async retry() {
      if (sessionId === null) {
        return null;
      }

      setIsMutating(true);
      setError(null);

      try {
        await retrySpawnSession(sessionId);
        setRefreshToken((current) => current + 1);
        return await refresh(sessionId);
      } catch (nextError: unknown) {
        setError(getErrorMessage(nextError, "Unknown birth session error."));
        return null;
      } finally {
        setIsMutating(false);
      }
    },
    async refund() {
      if (sessionId === null) {
        return null;
      }

      setIsMutating(true);
      setError(null);

      try {
        await refundSpawnSession(sessionId);
        setRefreshToken((current) => current + 1);
        return await refresh(sessionId);
      } catch (nextError: unknown) {
        setError(getErrorMessage(nextError, "Unknown birth session error."));
        return null;
      } finally {
        setIsMutating(false);
      }
    },
    reset() {
      setSessionId(null);
      setQuote(null);
      setDetail(null);
      setProvisionalSession(null);
      setError(null);
      setIsCreating(false);
      setIsRefreshing(false);
      setIsMutating(false);
      setRefreshToken(0);
    },
    formatAmount() {
      if (session === null) {
        return null;
      }

      return formatAssetAmount(session.grossAmount, session.asset);
    }
  };
}
