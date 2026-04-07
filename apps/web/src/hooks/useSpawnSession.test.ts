import { describe, expect, it } from "vitest";

import {
  deriveClaimId,
  type SpawnQuote,
  type SpawnSession,
  type SpawnSessionDetail
} from "@ic-automaton/shared";

import {
  derivePaymentInstructions,
  describeSpawnSessionProgress,
  formatSpawnSessionStateLabel
} from "./useSpawnSession";

function createSession(overrides: Partial<SpawnSession> = {}): SpawnSession {
  const sessionId = overrides.sessionId ?? "session-1";

  return {
    sessionId,
    claimId: deriveClaimId(sessionId),
    stewardAddress: "0xabc",
    chain: "base",
    asset: "usdc",
    grossAmount: "100",
    platformFee: "4.5",
    creationCost: "8",
    netForwardAmount: "87.5",
    quoteTermsHash: "0xdeadbeef",
    expiresAt: 1_709_912_400_000,
    state: "awaiting_payment",
    retryable: false,
    refundable: false,
    paymentStatus: "unpaid",
    automatonCanisterId: null,
    automatonEvmAddress: null,
    releaseTxHash: null,
    releaseBroadcastAt: null,
    parentId: null,
    childIds: [],
    selectedStrategies: [],
    config: {
      chain: "base",
      risk: 3,
      strategies: [],
      skills: [],
      provider: {
        openRouterApiKey: null,
        model: null,
        braveSearchApiKey: null,
        inferenceTransport: "openrouter_direct",
        openRouterReasoningLevel: "default"
      }
    },
    createdAt: 1_709_912_345_000,
    updatedAt: 1_709_912_345_000,
    ...overrides
  };
}

describe("useSpawnSession helpers", () => {
  it("describes retry and refund-eligible session states", () => {
    expect(
      describeSpawnSessionProgress(
        createSession({
          state: "failed",
          retryable: true,
          paymentStatus: "paid"
        })
      )
    ).toContain("retried");
    expect(
      describeSpawnSessionProgress(
        createSession({
          state: "expired",
          refundable: true,
          paymentStatus: "partial"
        })
      )
    ).toContain("Funds can now be reclaimed");
    expect(formatSpawnSessionStateLabel("payment_detected")).toBe("Payment Detected");
  });

  it("prefers cached payment instructions and falls back to the quote", () => {
    const session = createSession();
    const quote: SpawnQuote = {
      sessionId: session.sessionId,
      chain: session.chain,
      asset: session.asset,
      grossAmount: session.grossAmount,
      platformFee: session.platformFee,
      creationCost: session.creationCost,
      netForwardAmount: session.netForwardAmount,
      quoteTermsHash: session.quoteTermsHash,
      expiresAt: session.expiresAt,
      payment: {
        sessionId: session.sessionId,
        claimId: session.claimId,
        chain: session.chain,
        asset: session.asset,
        paymentAddress: "0xquote",
        grossAmount: session.grossAmount,
        quoteTermsHash: session.quoteTermsHash,
        expiresAt: session.expiresAt
      }
    };
    const detail: SpawnSessionDetail = {
      session,
      payment: {
        sessionId: session.sessionId,
        claimId: session.claimId,
        chain: session.chain,
        asset: session.asset,
        paymentAddress: "0xdetail",
        grossAmount: session.grossAmount,
        quoteTermsHash: session.quoteTermsHash,
        expiresAt: session.expiresAt
      },
      audit: [],
      registryRecord: null
    };

    expect(derivePaymentInstructions(session, detail, quote)).toMatchObject({
      claimId: session.claimId,
      paymentAddress: "0xdetail"
    });
    expect(derivePaymentInstructions(session, null, quote)).toMatchObject({
      claimId: session.claimId,
      paymentAddress: "0xquote"
    });
  });
});
