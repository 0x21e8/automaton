import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import {
  deriveSpawnJourneyProgress,
  getEffectivePendingReceipts,
  getPaymentSubmissionLockReason,
  SpawnWizard
} from "./SpawnWizard";

describe("SpawnWizard Genesis integration", () => {
  it("opens on the Genesis rite and blocks advancement until it is valid", () => {
    const markup = renderToStaticMarkup(
      <SpawnWizard
        isOpen
        onClose={() => {}}
        playgroundError={null}
        playgroundIsFallback={false}
        playgroundMetadata={null}
        walletSession={{
          address: null,
          chainId: null,
          hasProvider: false,
          isConnecting: false,
          isConnected: false,
          errorMessage: null,
          providers: [],
          selectedProviderId: null,
          selectedProviderName: null,
          walletLabel: "Wallet not detected",
          request: async () => {
            throw new Error("wallet request is not expected during static rendering");
          },
          connect: async () => {},
          disconnect: () => {},
          setSelectedProvider: () => {}
        }}
      />
    );

    expect(markup).toContain("Step 1 of 5");
    expect(markup).toContain("Genesis constitution");
    expect(markup).toContain("Name must be 1–64 characters.");
    expect(markup).toMatch(/<button[^>]*disabled=""[^>]*>Next<\/button>/);
    expect(markup).not.toContain("Risk appetite");
  });
});

describe("SpawnWizard pending receipt state", () => {
  it("keeps locally pending receipts while the session is still unpaid", () => {
    expect(
      getEffectivePendingReceipts(
        {
          state: "awaiting_payment",
          paymentStatus: "unpaid"
        },
        {
          approvalTxHash: "0xapprove",
          paymentTxHash: "0xdeposit",
          pendingReceipts: ["approval", "deposit"]
        }
      )
    ).toEqual(["approval", "deposit"]);
  });

  it("narrows pending receipts to the deposit after partial payment detection", () => {
    expect(
      getEffectivePendingReceipts(
        {
          state: "awaiting_payment",
          paymentStatus: "partial"
        },
        {
          approvalTxHash: "0xapprove",
          paymentTxHash: "0xdeposit",
          pendingReceipts: ["approval", "deposit"]
        }
      )
    ).toEqual(["deposit"]);
  });

  it("clears pending receipts once the session leaves the awaiting payment state", () => {
    expect(
      getEffectivePendingReceipts(
        {
          state: "spawning",
          paymentStatus: "paid"
        },
        {
          approvalTxHash: "0xapprove",
          paymentTxHash: "0xdeposit",
          pendingReceipts: ["approval", "deposit"]
        }
      )
    ).toEqual([]);
  });

  it("locks resubmission while locally submitted wallet transactions are still pending", () => {
    expect(
      getPaymentSubmissionLockReason(
        {
          state: "awaiting_payment",
          paymentStatus: "unpaid"
        },
        {
          approvalTxHash: "0xapprove",
          paymentTxHash: "0xdeposit",
          pendingReceipts: ["approval", "deposit"]
        },
        ["approval", "deposit"]
      )
    ).toContain("already submitted");
  });

  it("keeps the wallet action locked while waiting for the indexer to mirror confirmed payment", () => {
    expect(
      getPaymentSubmissionLockReason(
        {
          state: "awaiting_payment",
          paymentStatus: "unpaid"
        },
        {
          approvalTxHash: "0xapprove",
          paymentTxHash: "0xdeposit",
          pendingReceipts: []
        },
        []
      )
    ).toContain("mirror");
  });
});

describe("deriveSpawnJourneyProgress", () => {
  it("shows session creation progress immediately after spawn is clicked", () => {
    const progress = deriveSpawnJourneyProgress(null, null, [], true, false);

    expect(progress?.currentLabel).toBe("Creating factory session");
    expect(progress?.steps[0]).toMatchObject({
      key: "session",
      status: "current"
    });
  });

  it("keeps the wallet step current while wallet transactions are still pending", () => {
    const progress = deriveSpawnJourneyProgress(
      {
        state: "awaiting_payment",
        paymentStatus: "unpaid",
        refundable: false,
        retryable: false
      },
      {
        approvalTxHash: "0xapprove",
        paymentTxHash: "0xdeposit",
        pendingReceipts: ["approval", "deposit"]
      },
      ["approval", "deposit"],
      false,
      false
    );

    expect(progress?.currentLabel).toBe("Confirming on-chain payment");
    expect(progress?.steps[1]).toMatchObject({
      key: "payment",
      status: "current"
    });
  });

  it("moves to factory confirmation after wallet transactions are confirmed", () => {
    const progress = deriveSpawnJourneyProgress(
      {
        state: "awaiting_payment",
        paymentStatus: "unpaid",
        refundable: false,
        retryable: false
      },
      {
        approvalTxHash: "0xapprove",
        paymentTxHash: "0xdeposit",
        pendingReceipts: []
      },
      [],
      false,
      false
    );

    expect(progress?.currentLabel).toBe("Waiting for factory confirmation");
    expect(progress?.steps[1]).toMatchObject({
      key: "payment",
      status: "complete"
    });
    expect(progress?.steps[2]).toMatchObject({
      key: "detection",
      status: "current"
    });
  });

  it("marks provisioning as current once payment is detected", () => {
    const progress = deriveSpawnJourneyProgress(
      {
        state: "spawning",
        paymentStatus: "paid",
        refundable: false,
        retryable: false
      },
      null,
      [],
      false,
      false
    );

    expect(progress?.currentLabel).toBe("Provisioning automaton");
    expect(progress?.steps[0]?.status).toBe("complete");
    expect(progress?.steps[1]?.status).toBe("complete");
    expect(progress?.steps[2]?.status).toBe("complete");
    expect(progress?.steps[3]).toMatchObject({
      key: "provision",
      status: "current"
    });
  });

  it("marks every step complete once the spawn finishes", () => {
    const progress = deriveSpawnJourneyProgress(
      {
        state: "complete",
        paymentStatus: "paid",
        refundable: false,
        retryable: false
      },
      null,
      [],
      false,
      false
    );

    expect(progress?.currentLabel).toBe("Birth complete");
    expect(progress?.completedCount).toBe(progress?.totalCount);
    expect(progress?.steps.every((step) => step.status === "complete")).toBe(true);
  });
});
