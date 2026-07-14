import { describe, expect, it } from "vitest";
import { buildPaymentGraph, parseJournalPaymentClaim, parsePeerPaymentClaim, settleRoomMessage, settleRoomMessagesBatch, settleSocietyClaims } from "../src/lib/society.js";
import { createRoomMessageFixture, createSpawnedAutomatonRecordFixture } from "./fixtures.js";

const txHash = `0x${"ab".repeat(32)}`;
const payer = createSpawnedAutomatonRecordFixture({ canisterId: "payer-cai", evmAddress: "0x1111111111111111111111111111111111111111" });
const payee = createSpawnedAutomatonRecordFixture({ canisterId: "payee-cai", evmAddress: "0x2222222222222222222222222222222222222222" });
const claim = createRoomMessageFixture({ authorCanisterId: payer.canisterId, mentions: [payee.canisterId], contentType: "application/json", body: JSON.stringify({ kind: "peer_payment_claim", version: 1, tx_hash: txHash, peer_canister_id: payee.canisterId, asset: "eth", amount_raw: "25" }) });

describe("said-vs-paid anchoring", () => {
  it("only parses the explicit bounded claim schema", () => {
    expect(parsePeerPaymentClaim(claim)?.txHash).toBe(txHash);
    expect(parsePeerPaymentClaim({ ...claim, contentType: "text/plain", body: `paid ${txHash}` })).toBeNull();
  });

  it("parses only explicit journal companion claims, never journal prose", () => {
    const entry = { id: 1, turnId: "turn-1", timestamp: 100, text: `I paid ${txHash}`, genesis: false };
    expect(parseJournalPaymentClaim(entry)).toBeNull();
    expect(parseJournalPaymentClaim({ ...entry, dealClaim: { kind: "peer_payment_claim", version: 1, txHash, peerCanisterId: payee.canisterId, asset: "eth", amountRaw: "25" } })?.txHash).toBe(txHash);
    expect(parseJournalPaymentClaim({ ...entry, dealClaim: { kind: "peer_payment_claim", version: 2, txHash, peerCanisterId: payee.canisterId, asset: "eth", amountRaw: "25" } as never })).toBeNull();
  });

  it("verifies journal claims and globally deduplicates the same transaction across room and journal", async () => {
    const verifier = { getReceiptStatus: async () => true, getTransaction: async () => ({ hash: txHash, from: payer.evmAddress, to: payee.evmAddress, value: "0x19", input: "0x", blockNumber: "0x10" }) };
    const journalEntry = { id: 1, turnId: "turn-1", timestamp: claim.createdAt + 1, text: "I submitted the agreed payment.", genesis: false, dealClaim: { kind: "peer_payment_claim" as const, version: 1 as const, txHash, peerCanisterId: payee.canisterId, asset: "eth" as const, amountRaw: "25" } };
    const journalOnly = await settleSocietyClaims([], [{ canisterId: payer.canisterId, entries: [journalEntry] }], [payer, payee], verifier, 123);
    expect(journalOnly.journals[0]?.entries[0]?.settlement?.status).toBe("settled");
    expect(buildPaymentGraph(journalOnly.paymentEvents)).toMatchObject([{ amountRaw: "25", transactionCount: 1, txHashes: [txHash] }]);

    const duplicate = await settleSocietyClaims([claim], [{ canisterId: payer.canisterId, entries: [journalEntry] }], [payer, payee], verifier, 123);
    expect(duplicate.roomMessages[0]?.settlement?.status).toBe("settled");
    expect(duplicate.journals[0]?.entries[0]?.settlement?.status).toBe("unsettled");
    expect(buildPaymentGraph(duplicate.paymentEvents)).toMatchObject([{ amountRaw: "25", transactionCount: 1, txHashes: [txHash] }]);
  });

  it("under-matches journal claims with the wrong sender or amount", async () => {
    const entry = { id: 1, turnId: "turn-1", timestamp: 100, text: "I submitted a payment.", genesis: false, dealClaim: { kind: "peer_payment_claim" as const, version: 1 as const, txHash, peerCanisterId: payee.canisterId, asset: "eth" as const, amountRaw: "25" } };
    for (const transaction of [
      { hash: txHash, from: payee.evmAddress, to: payee.evmAddress, value: "0x19", input: "0x", blockNumber: "0x10" },
      { hash: txHash, from: payer.evmAddress, to: payee.evmAddress, value: "0x18", input: "0x", blockNumber: "0x10" }
    ]) {
      const result = await settleSocietyClaims([], [{ canisterId: payer.canisterId, entries: [entry] }], [payer, payee], { getReceiptStatus: async () => true, getTransaction: async () => transaction }, 123);
      expect(result.journals[0]?.entries[0]?.settlement?.status).toBe("unsettled");
      expect(result.paymentEvents).toHaveLength(0);
    }
  });

  it("settles only a confirmed exact registry-address transfer", async () => {
    const verifier = { getReceiptStatus: async () => true, getTransaction: async () => ({ hash: txHash, from: payer.evmAddress, to: payee.evmAddress, value: "0x19", input: "0x", blockNumber: "0x10" }) };
    const settled = await settleRoomMessage(claim, [payer, payee], verifier, 123);
    expect(settled).toMatchObject({ status: "settled", amountRaw: "25", verifiedAt: 123 });
    expect(buildPaymentGraph([{ ...claim, settlement: settled }])).toEqual([{ fromCanisterId: payer.canisterId, toCanisterId: payee.canisterId, asset: "eth", amountRaw: "25", transactionCount: 1, txHashes: [txHash] }]);
  });

  it("settles a transaction hash only once, at its earliest valid claim", async () => {
    const verifier = { getReceiptStatus: async () => true, getTransaction: async () => ({ hash: txHash, from: payer.evmAddress, to: payee.evmAddress, value: "0x19", input: "0x", blockNumber: "0x10" }) };
    const messages = await settleRoomMessagesBatch([
      { ...claim, messageId: "second", seq: 2 },
      { ...claim, messageId: "first", seq: 1 }
    ], [payer, payee], verifier, 123);
    expect(messages.map((message) => [message.messageId, message.settlement?.status])).toEqual([
      ["first", "settled"],
      ["second", "unsettled"]
    ]);
    expect(buildPaymentGraph(messages)).toMatchObject([{ amountRaw: "25", transactionCount: 1, txHashes: [txHash] }]);
  });

  it("settles a paid-Inbox call only when the Inbox address is independently trusted", async () => {
    const inbox = "0x4444444444444444444444444444444444444444";
    const recipientWord = payee.evmAddress.slice(2).padStart(64, "0");
    const input = `0xdc0a1b6a${recipientWord}${(96n).toString(16).padStart(64, "0")}${"0".repeat(64)}${(1n).toString(16).padStart(64, "0")}${"78"}${"0".repeat(62)}`;
    const verifier = { getReceiptStatus: async () => true, getTransaction: async () => ({ hash: txHash, from: payer.evmAddress, to: inbox, value: "0x19", input, blockNumber: "0x10" }) };
    await expect(settleRoomMessage(claim, [payer, payee], verifier, 123)).resolves.toMatchObject({ status: "unsettled" });
    await expect(settleRoomMessage(claim, [payer, payee], verifier, 123, new Set([inbox]))).resolves.toMatchObject({ status: "settled" });
  });

  it("under-matches wrong recipient, amount, sender, missing mention, and failed receipts", async () => {
    const variants = [
      { from: payer.evmAddress, to: "0x3333333333333333333333333333333333333333", value: "0x19", input: "0x", blockNumber: "0x10" },
      { from: payer.evmAddress, to: payee.evmAddress, value: "0x18", input: "0x", blockNumber: "0x10" },
      { from: payee.evmAddress, to: payee.evmAddress, value: "0x19", input: "0x", blockNumber: "0x10" }
    ];
    for (const tx of variants) await expect(settleRoomMessage(claim, [payer, payee], { getReceiptStatus: async () => true, getTransaction: async () => ({ hash: txHash, ...tx }) })).resolves.toMatchObject({ status: "unsettled" });
    await expect(settleRoomMessage({ ...claim, mentions: [] }, [payer, payee], { getReceiptStatus: async () => true, getTransaction: async () => null })).resolves.toMatchObject({ status: "unsettled" });
    await expect(settleRoomMessage(claim, [payer, payee], { getReceiptStatus: async () => false, getTransaction: async () => ({ hash: txHash, ...variants[0] }) })).resolves.toMatchObject({ status: "unsettled" });
  });
});
