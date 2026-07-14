import type {
  JournalEntry,
  PaymentGraphEdge,
  RoomMessage,
  RoomMessageSettlement,
  SpawnedAutomatonRecord
} from "@ic-automaton/shared";

const TX_HASH = /^0x[0-9a-f]{64}$/;
const ADDRESS = /^0x[0-9a-f]{40}$/;
const ERC20_TRANSFER_SELECTOR = "a9059cbb";
const QUEUE_MESSAGE_SELECTOR = "dc0a1b6a";

export interface PeerPaymentClaim {
  kind: "peer_payment_claim";
  version: 1;
  txHash: string;
  peerCanisterId: string;
  asset: "eth" | "usdc";
  amountRaw: string;
}

export interface ChainTransaction {
  hash: string;
  from: string;
  to: string;
  value: string;
  input: string;
  blockNumber: string | null;
}

export interface TransactionVerifier {
  getTransaction(txHash: string): Promise<ChainTransaction | null>;
  getReceiptStatus(txHash: string): Promise<boolean>;
}

function normalizeAddress(value: string) {
  return value.toLowerCase();
}

function parseQuantity(value: string) {
  try {
    return BigInt(value).toString();
  } catch {
    return null;
  }
}

export function parsePeerPaymentClaim(message: RoomMessage): PeerPaymentClaim | null {
  if (message.contentType !== "application/json") return null;
  let value: unknown;
  try {
    value = JSON.parse(message.body);
  } catch {
    return null;
  }
  if (!value || typeof value !== "object" || Array.isArray(value)) return null;
  const record = value as Record<string, unknown>;
  if (
    record.kind !== "peer_payment_claim" ||
    record.version !== 1 ||
    typeof record.tx_hash !== "string" ||
    typeof record.peer_canister_id !== "string" ||
    (record.asset !== "eth" && record.asset !== "usdc") ||
    typeof record.amount_raw !== "string" ||
    !/^\d+$/.test(record.amount_raw) ||
    !TX_HASH.test(record.tx_hash.toLowerCase())
  ) return null;
  return {
    kind: "peer_payment_claim",
    version: 1,
    txHash: record.tx_hash.toLowerCase(),
    peerCanisterId: record.peer_canister_id,
    asset: record.asset,
    amountRaw: BigInt(record.amount_raw).toString()
  };
}

export function parseJournalPaymentClaim(entry: JournalEntry): PeerPaymentClaim | null {
  const claim = entry.dealClaim as unknown;
  if (!claim || typeof claim !== "object" || Array.isArray(claim)) return null;
  const record = claim as Record<string, unknown>;
  if (
    record.kind !== "peer_payment_claim" ||
    record.version !== 1 ||
    typeof record.txHash !== "string" ||
    typeof record.peerCanisterId !== "string" ||
    (record.asset !== "eth" && record.asset !== "usdc") ||
    typeof record.amountRaw !== "string" ||
    !/^\d+$/.test(record.amountRaw) ||
    !TX_HASH.test(record.txHash.toLowerCase())
  ) return null;
  return {
    kind: "peer_payment_claim",
    version: 1,
    txHash: record.txHash.toLowerCase(),
    peerCanisterId: record.peerCanisterId,
    asset: record.asset,
    amountRaw: BigInt(record.amountRaw).toString()
  };
}

function claimAsRoomMessage(id: string, authorCanisterId: string, createdAt: number, claim: PeerPaymentClaim): RoomMessage {
  return {
    messageId: id,
    seq: 0,
    authorCanisterId,
    createdAt,
    body: JSON.stringify({ kind: claim.kind, version: claim.version, tx_hash: claim.txHash, peer_canister_id: claim.peerCanisterId, asset: claim.asset, amount_raw: claim.amountRaw }),
    mentions: [claim.peerCanisterId],
    contentType: "application/json"
  };
}

export async function settleSocietyClaims(
  roomMessages: ReadonlyArray<RoomMessage>,
  journals: ReadonlyArray<{ canisterId: string; entries: JournalEntry[] }>,
  registry: ReadonlyArray<SpawnedAutomatonRecord>,
  verifier: TransactionVerifier,
  now = Date.now(),
  trustedInboxAddresses: ReadonlySet<string> = new Set(),
  trustedUsdcAddresses: ReadonlySet<string> = new Set()
) {
  type Envelope = { source: "room" | "journal"; key: string; createdAt: number; order: number; message: RoomMessage };
  const envelopes: Envelope[] = [];
  for (const message of roomMessages) {
    if (parsePeerPaymentClaim(message)) envelopes.push({ source: "room", key: message.messageId, createdAt: message.createdAt, order: message.seq, message });
  }
  for (const journal of journals) for (const entry of journal.entries) {
    const claim = parseJournalPaymentClaim(entry);
    if (claim) envelopes.push({ source: "journal", key: `${journal.canisterId}:${entry.id}`, createdAt: entry.timestamp, order: entry.id, message: claimAsRoomMessage(`journal:${journal.canisterId}:${entry.id}`, journal.canisterId, entry.timestamp, claim) });
  }
  envelopes.sort((left, right) => left.createdAt - right.createdAt || (left.source === right.source ? 0 : left.source === "room" ? -1 : 1) || left.order - right.order || left.key.localeCompare(right.key));
  const usedHashes = new Set<string>();
  const roomSettlements = new Map<string, RoomMessageSettlement>();
  const journalSettlements = new Map<string, RoomMessageSettlement>();
  const paymentEvents: RoomMessage[] = [];
  for (const envelope of envelopes) {
    const claim = parsePeerPaymentClaim(envelope.message)!;
    let settlement: RoomMessageSettlement;
    if (usedHashes.has(claim.txHash)) {
      settlement = { status: "unsettled", txHash: claim.txHash, payerCanisterId: envelope.message.authorCanisterId, payeeCanisterId: claim.peerCanisterId, asset: claim.asset, amountRaw: claim.amountRaw, verifiedAt: null, provenance: null };
    } else {
      settlement = await settleRoomMessage(envelope.message, registry, verifier, now, trustedInboxAddresses, trustedUsdcAddresses).catch(() => ({ status: "unsettled", txHash: claim.txHash, payerCanisterId: envelope.message.authorCanisterId, payeeCanisterId: claim.peerCanisterId, asset: claim.asset, amountRaw: claim.amountRaw, verifiedAt: null, provenance: null } as RoomMessageSettlement));
      if (settlement.status === "settled") usedHashes.add(claim.txHash);
    }
    (envelope.source === "room" ? roomSettlements : journalSettlements).set(envelope.key, settlement);
    if (settlement.status === "settled") paymentEvents.push({ ...envelope.message, settlement });
  }
  return {
    roomMessages: roomMessages.map((message) => roomSettlements.has(message.messageId) ? { ...message, settlement: roomSettlements.get(message.messageId) } : message),
    journals: journals.map((journal) => ({ canisterId: journal.canisterId, entries: journal.entries.map((entry) => journalSettlements.has(`${journal.canisterId}:${entry.id}`) ? { ...entry, settlement: journalSettlements.get(`${journal.canisterId}:${entry.id}`) } : entry) })),
    paymentEvents
  };
}

function decodeErc20Transfer(input: string) {
  const data = input.toLowerCase().replace(/^0x/, "");
  if (data.length !== 8 + 64 + 64 || !data.startsWith(ERC20_TRANSFER_SELECTOR)) return null;
  const address = `0x${data.slice(8 + 24, 8 + 64)}`;
  const amount = parseQuantity(`0x${data.slice(8 + 64)}`);
  return ADDRESS.test(address) && amount !== null ? { address, amount } : null;
}

function decodeQueueMessage(input: string) {
  const data = input.toLowerCase().replace(/^0x/, "");
  if (data.length < 8 + 64 * 4 || !data.startsWith(QUEUE_MESSAGE_SELECTOR)) return null;
  const recipient = `0x${data.slice(8 + 24, 8 + 64)}`;
  const offset = parseQuantity(`0x${data.slice(8 + 64, 8 + 128)}`);
  const usdcAmount = parseQuantity(`0x${data.slice(8 + 128, 8 + 192)}`);
  return ADDRESS.test(recipient) && offset === "96" && usdcAmount !== null ? { recipient, usdcAmount } : null;
}

export async function settleRoomMessage(
  message: RoomMessage,
  registry: ReadonlyArray<SpawnedAutomatonRecord>,
  verifier: TransactionVerifier,
  now = Date.now(),
  trustedInboxAddresses: ReadonlySet<string> = new Set(),
  trustedUsdcAddresses: ReadonlySet<string> = new Set()
): Promise<RoomMessageSettlement> {
  const claim = parsePeerPaymentClaim(message);
  if (!claim) return { status: "not_claimed", txHash: null, payerCanisterId: null, payeeCanisterId: null, asset: null, amountRaw: null, verifiedAt: null, provenance: null };
  const payer = registry.find((entry) => entry.canisterId === message.authorCanisterId);
  const payee = registry.find((entry) => entry.canisterId === claim.peerCanisterId);
  const base = { txHash: claim.txHash, payerCanisterId: payer?.canisterId ?? null, payeeCanisterId: payee?.canisterId ?? null, asset: claim.asset, amountRaw: claim.amountRaw } as const;
  if (!payer || !payee || !message.mentions.includes(payee.canisterId)) {
    return { status: "unsettled", ...base, verifiedAt: null, provenance: null };
  }
  const [tx, succeeded] = await Promise.all([
    verifier.getTransaction(claim.txHash),
    verifier.getReceiptStatus(claim.txHash)
  ]);
  if (!tx || !succeeded || tx.blockNumber === null || normalizeAddress(tx.from) !== normalizeAddress(payer.evmAddress)) {
    return { status: "unsettled", ...base, verifiedAt: null, provenance: null };
  }
  let matches = false;
  if (claim.asset === "eth") {
    const inboxCall = decodeQueueMessage(tx.input);
    const direct = normalizeAddress(tx.to) === normalizeAddress(payee.evmAddress) && tx.input.toLowerCase() === "0x";
    const paidInbox = inboxCall !== null && inboxCall.usdcAmount === "0" && normalizeAddress(inboxCall.recipient) === normalizeAddress(payee.evmAddress) && trustedInboxAddresses.has(normalizeAddress(tx.to));
    matches = (direct || paidInbox) && parseQuantity(tx.value) === claim.amountRaw;
  } else {
    const transfer = decodeErc20Transfer(tx.input);
    matches = transfer !== null && trustedUsdcAddresses.has(normalizeAddress(tx.to)) && normalizeAddress(transfer.address) === normalizeAddress(payee.evmAddress) && transfer.amount === claim.amountRaw && parseQuantity(tx.value) === "0";
  }
  return matches
    ? { status: "settled", ...base, verifiedAt: now, provenance: `/api/society/transactions/${claim.txHash}` }
    : { status: "unsettled", ...base, verifiedAt: null, provenance: null };
}

export async function settleRoomMessagesBatch(
  messages: ReadonlyArray<RoomMessage>,
  registry: ReadonlyArray<SpawnedAutomatonRecord>,
  verifier: TransactionVerifier,
  now = Date.now(),
  trustedInboxAddresses: ReadonlySet<string> = new Set(),
  trustedUsdcAddresses: ReadonlySet<string> = new Set()
) {
  const settledHashes = new Set<string>();
  const results: RoomMessage[] = [];
  for (const message of [...messages].sort((left, right) => left.seq - right.seq)) {
    const claim = parsePeerPaymentClaim(message);
    if (claim && settledHashes.has(claim.txHash)) {
      results.push({ ...message, settlement: { status: "unsettled", txHash: claim.txHash, payerCanisterId: message.authorCanisterId, payeeCanisterId: claim.peerCanisterId, asset: claim.asset, amountRaw: claim.amountRaw, verifiedAt: null, provenance: null } });
      continue;
    }
    const settlement = await settleRoomMessage(message, registry, verifier, now, trustedInboxAddresses, trustedUsdcAddresses);
    if (settlement.status === "settled" && settlement.txHash) settledHashes.add(settlement.txHash);
    results.push(settlement.status === "not_claimed" ? message : { ...message, settlement });
  }
  return results;
}

export function buildPaymentGraph(messages: ReadonlyArray<RoomMessage>): PaymentGraphEdge[] {
  const grouped = new Map<string, PaymentGraphEdge>();
  for (const message of messages) {
    const settlement = message.settlement;
    if (settlement?.status !== "settled" || !settlement.payerCanisterId || !settlement.payeeCanisterId || !settlement.asset || !settlement.amountRaw || !settlement.txHash) continue;
    const key = `${settlement.payerCanisterId}:${settlement.payeeCanisterId}:${settlement.asset}`;
    const edge = grouped.get(key) ?? { fromCanisterId: settlement.payerCanisterId, toCanisterId: settlement.payeeCanisterId, asset: settlement.asset, amountRaw: "0", transactionCount: 0, txHashes: [] };
    edge.amountRaw = (BigInt(edge.amountRaw) + BigInt(settlement.amountRaw)).toString();
    edge.transactionCount += 1;
    edge.txHashes.push(settlement.txHash);
    grouped.set(key, edge);
  }
  return [...grouped.values()].sort((a, b) => `${a.fromCanisterId}:${a.toCanisterId}:${a.asset}`.localeCompare(`${b.fromCanisterId}:${b.toCanisterId}:${b.asset}`));
}

export function createJsonRpcTransactionVerifier(rpcUrl: string, fetchImpl: typeof fetch = fetch): TransactionVerifier {
  let id = 0;
  const call = async (method: string, params: unknown[]) => {
    const response = await fetchImpl(rpcUrl, { method: "POST", headers: { "content-type": "application/json" }, body: JSON.stringify({ jsonrpc: "2.0", id: ++id, method, params }) });
    if (!response.ok) throw new Error(`EVM RPC ${method} failed with HTTP ${response.status}`);
    const body = await response.json() as { result?: unknown; error?: { message?: string } };
    if (body.error) throw new Error(body.error.message ?? `EVM RPC ${method} failed`);
    return body.result;
  };
  return {
    async getTransaction(txHash) { return await call("eth_getTransactionByHash", [txHash]) as ChainTransaction | null; },
    async getReceiptStatus(txHash) {
      const receipt = await call("eth_getTransactionReceipt", [txHash]) as { status?: string } | null;
      return receipt?.status === "0x1";
    }
  };
}
