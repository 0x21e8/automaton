import type {
  AutomatonDetail,
  JournalEntry,
  MonologueEntry,
  RealtimeEvent,
  RoomMessage,
  SpawnedAutomatonRecord
} from "@ic-automaton/shared";

import type { IndexerConfig } from "../config.js";
import type { AutomatonClient } from "../integrations/automaton-client.js";
import type { FactoryClient } from "../integrations/factory-client.js";
import { FaucetError, type FaucetService } from "../lib/faucet.js";
import { diffAutomatonRecord } from "../lib/automaton-record.js";
import { normalizeAutomatonDetail, normalizeMonologueEntries } from "../normalize/automaton.js";
import type { IndexerStore } from "../store/sqlite.js";

const ROOM_POLL_PAGE_LIMIT = 100;
const ROOM_RETENTION_MS = 7 * 24 * 60 * 60 * 1000;

export interface EthUsdPriceSourceSnapshot {
  ethUsd: number | null;
  label: string;
  source: "fixed";
  updatedAt: number;
}

export interface EthUsdPriceSource {
  read(): Promise<EthUsdPriceSourceSnapshot>;
}

export class FixedEthUsdPriceSource implements EthUsdPriceSource {
  constructor(private readonly ethUsd = 2_500) {}

  async read(): Promise<EthUsdPriceSourceSnapshot> {
    return {
      ethUsd: this.ethUsd,
      source: "fixed",
      label: `fixed:${this.ethUsd}`,
      updatedAt: Date.now()
    };
  }
}

export interface PollRunSnapshot {
  failureCount: number;
  inFlight: boolean;
  lastAttemptAt: number | null;
  lastDurationMs: number | null;
  lastError: string | null;
  lastSuccessAt: number | null;
  successCount: number;
}

export interface CanisterPollSnapshot {
  currentDetailAvailable: boolean;
  identity: PollRunSnapshot;
  lastIndexedMonologueCount: number;
  lastIndexedJournalCount: number;
  lastObservedTurnId: string | null;
  lastPersistedAt: number | null;
  monologue: PollRunSnapshot;
  journal: PollRunSnapshot;
  runtime: PollRunSnapshot;
}

export interface AutomatonIndexerSnapshot {
  canisters: Record<string, CanisterPollSnapshot>;
  enabled: boolean;
  price: EthUsdPriceSourceSnapshot | null;
  startedAt: number | null;
}

function createPollRunSnapshot(): PollRunSnapshot {
  return {
    inFlight: false,
    lastAttemptAt: null,
    lastSuccessAt: null,
    lastDurationMs: null,
    lastError: null,
    successCount: 0,
    failureCount: 0
  };
}

function createCanisterPollSnapshot(): CanisterPollSnapshot {
  return {
    identity: createPollRunSnapshot(),
    runtime: createPollRunSnapshot(),
    monologue: createPollRunSnapshot(),
    journal: createPollRunSnapshot(),
    lastPersistedAt: null,
    lastIndexedMonologueCount: 0,
    lastIndexedJournalCount: 0,
    lastObservedTurnId: null,
    currentDetailAvailable: false
  };
}

export interface AutomatonIndexerOptions {
  client: AutomatonClient;
  config: IndexerConfig;
  eventPublisher?: RealtimeEventPublisher;
  factoryClient?: Pick<
    FactoryClient,
    "isConfigured" | "listRoomMessages" | "listSpawnedAutomatons"
  > &
    Partial<Pick<FactoryClient, "getSpawnSession">>;
  faucetService?: FaucetService;
  priceSource?: EthUsdPriceSource;
  store: IndexerStore;
}

export interface RealtimeEventPublisher {
  broadcast(event: RealtimeEvent): void;
}

export class AutomatonIndexer {
  private readonly client: AutomatonClient;
  private readonly config: IndexerConfig;
  private eventPublisher?: RealtimeEventPublisher;
  private readonly factoryClient?: Pick<
    FactoryClient,
    "isConfigured" | "listRoomMessages" | "listSpawnedAutomatons"
  > &
    Partial<Pick<FactoryClient, "getSpawnSession">>;
  private readonly faucetService?: FaucetService;
  private readonly priceSource: EthUsdPriceSource;
  private readonly store: IndexerStore;
  private factoryDiscoveryInFlight = false;
  private roomPollInFlight = false;

  private readonly snapshot: AutomatonIndexerSnapshot = {
    startedAt: null,
    enabled: false,
    price: null,
    canisters: {}
  };

  private readonly timers = new Set<NodeJS.Timeout>();

  constructor(options: AutomatonIndexerOptions) {
    this.client = options.client;
    this.config = options.config;
    this.store = options.store;
    this.factoryClient = options.factoryClient;
    this.faucetService = options.faucetService;
    this.priceSource = options.priceSource ?? new FixedEthUsdPriceSource();
    this.eventPublisher = options.eventPublisher;
  }

  setEventPublisher(eventPublisher: RealtimeEventPublisher | undefined) {
    this.eventPublisher = eventPublisher;
  }

  start() {
    if (this.snapshot.enabled) {
      return;
    }

    this.snapshot.enabled = true;
    this.snapshot.startedAt = Date.now();
    void this.refreshPriceNow().catch(() => undefined);
    this.schedule(this.config.slowPollIntervalMs, () => this.pollIdentityNow());
    this.schedule(this.config.fastPollIntervalMs, () => this.pollRuntimeNow());
    this.schedule(this.config.fastPollIntervalMs, () => this.pollMonologueNow());
    this.schedule(this.config.fastPollIntervalMs, () => this.pollJournalNow());
    this.schedule(this.config.pricePollIntervalMs, () => this.refreshPriceNow());
    this.schedule(this.config.fastPollIntervalMs, () => this.syncFactoryRegistryNow());
    this.schedule(this.config.slowPollIntervalMs, () => this.pollRoomNow());
  }

  async stop() {
    this.snapshot.enabled = false;

    for (const timer of this.timers) {
      clearInterval(timer);
    }

    this.timers.clear();
  }

  getSnapshot(): AutomatonIndexerSnapshot {
    return {
      startedAt: this.snapshot.startedAt,
      enabled: this.snapshot.enabled,
      price: this.snapshot.price ? { ...this.snapshot.price } : null,
      canisters: Object.fromEntries(
        Object.entries(this.snapshot.canisters).map(([canisterId, entry]) => [
          canisterId,
          {
            ...entry,
            identity: { ...entry.identity },
            runtime: { ...entry.runtime },
            monologue: { ...entry.monologue },
            journal: { ...entry.journal }
          }
        ])
      )
    };
  }

  async refreshPriceNow() {
    const price = await this.priceSource.read();
    this.snapshot.price = price;
    await this.store.setPrice("ethUsd", price.ethUsd);
  }

  async pollIdentityNow() {
    for (const canisterId of await this.listTrackedCanisterIds()) {
      await this.pollIdentityFor(canisterId);
    }
  }

  async pollRuntimeNow() {
    for (const canisterId of await this.listTrackedCanisterIds()) {
      await this.pollRuntimeFor(canisterId);
    }
  }

  async pollMonologueNow() {
    for (const canisterId of await this.listTrackedCanisterIds()) {
      await this.pollMonologueFor(canisterId);
    }
  }

  async pollJournalNow() {
    if (!this.client.readJournal) return;
    for (const canisterId of await this.listTrackedCanisterIds()) {
      await this.pollJournalFor(canisterId);
    }
  }

  async syncFactoryRegistryNow() {
    if (!this.factoryClient?.isConfigured() || this.factoryDiscoveryInFlight) {
      return;
    }

    this.factoryDiscoveryInFlight = true;

    try {
      const trackedBefore = new Set(await this.listTrackedCanisterIds());
      const records = await this.readFactoryRegistry();
      await this.store.replaceSpawnedAutomatonRegistry(records);

      if (this.factoryClient.getSpawnSession) {
        for (const record of records) {
          const detail = await this.factoryClient.getSpawnSession(record.sessionId);
          if (!detail) {
            await this.autoFundSpawnedAutomaton(record.sessionId, record.evmAddress);
            continue;
          }

          await this.store.upsertSpawnSession({
            session: detail.session,
            payment: detail.payment,
            audit: detail.audit,
            registryRecord: record
          });
          await this.autoFundSpawnedAutomaton(
            detail.session.sessionId,
            detail.registryRecord?.evmAddress ?? detail.session.automatonEvmAddress ?? record.evmAddress
          );
        }
      } else {
        for (const record of records) {
          await this.autoFundSpawnedAutomaton(record.sessionId, record.evmAddress);
        }
      }

      const newlyDiscovered = new Set(
        records
          .map((record) => record.canisterId)
          .filter((canisterId) => !trackedBefore.has(canisterId))
      );
      for (const canisterId of newlyDiscovered) {
        await this.hydrateNewlyDiscoveredCanister(canisterId);
      }
    } finally {
      this.factoryDiscoveryInFlight = false;
    }
  }

  async pollRoomNow() {
    if (!this.factoryClient?.isConfigured() || this.roomPollInFlight) {
      return;
    }

    this.roomPollInFlight = true;

    try {
      let afterSeq = await this.store.getLatestRoomMessageSeq();

      while (true) {
        const page = await this.factoryClient.listRoomMessages(
          afterSeq ?? undefined,
          ROOM_POLL_PAGE_LIMIT
        );

        await this.store.upsertRoomMessages(page.messages, page.latestSeq);
        await this.store.pruneRoomMessages(Date.now() - ROOM_RETENTION_MS);

        for (const message of this.selectNewRoomMessages(page.messages, afterSeq)) {
          this.eventPublisher?.broadcast({
            type: "message",
            message
          });
        }

        if (page.nextAfterSeq === null) {
          break;
        }

        afterSeq = page.nextAfterSeq;
      }
    } finally {
      this.roomPollInFlight = false;
    }
  }

  private async autoFundSpawnedAutomaton(
    sessionId: string,
    walletAddress: string | null | undefined
  ) {
    if (
      !this.config.playground.metadata.faucet.available ||
      !this.faucetService ||
      !walletAddress
    ) {
      return;
    }

    try {
      await this.faucetService.claim({
        ipAddress: `automaton:${sessionId}`,
        walletAddress
      });
    } catch (error) {
      if (error instanceof FaucetError && error.statusCode === 429) {
        return;
      }
    }
  }

  private async hydrateNewlyDiscoveredCanister(canisterId: string) {
    await this.pollIdentityFor(canisterId);
    await this.pollRuntimeFor(canisterId);
    await this.pollMonologueFor(canisterId);
    await this.pollJournalFor(canisterId);
  }

  private async pollIdentityFor(canisterId: string) {
    await this.runPoll(canisterId, "identity", async () => {
      const existingDetail = await this.store.getAutomatonDetail(canisterId);
      const registryRecord = await this.store.getSpawnedAutomatonRegistryRecord(canisterId);
      const spawnSession =
        registryRecord === null
          ? null
          : await this.store.getSpawnSessionDetail(registryRecord.sessionId);
      const identity = await this.client.readIdentityConfig(canisterId);
      const detail = normalizeAutomatonDetail({
        canisterId,
        config: this.config.ingestion,
        existingDetail,
        identity,
        now: Date.now(),
        registryRecord,
        spawnModel: spawnSession?.session.config.provider.model ?? null,
        ethUsd: this.snapshot.price?.ethUsd ?? null
      });

      await this.persistDetail(canisterId, existingDetail, detail);
    });
  }

  private async pollRuntimeFor(canisterId: string) {
    await this.runPoll(canisterId, "runtime", async () => {
      const existingDetail = await this.store.getAutomatonDetail(canisterId);
      const registryRecord = await this.store.getSpawnedAutomatonRegistryRecord(canisterId);
      const spawnSession =
        registryRecord === null
          ? null
          : await this.store.getSpawnSessionDetail(registryRecord.sessionId);
      const runtime = await this.client.readRuntimeFinancial(canisterId);
      const detail = normalizeAutomatonDetail({
        canisterId,
        config: this.config.ingestion,
        existingDetail,
        now: Date.now(),
        registryRecord,
        runtime,
        spawnModel: spawnSession?.session.config.provider.model ?? null,
        ethUsd: this.snapshot.price?.ethUsd ?? null
      });

      await this.persistDetail(canisterId, existingDetail, detail);
    });
  }

  private async pollMonologueFor(canisterId: string) {
    await this.runPoll(canisterId, "monologue", async () => {
      const turns = await this.client.readRecentTurns(canisterId);
      const entries = normalizeMonologueEntries(turns.recentTurns);
      const existingEntries = await this.store.listMonologue(canisterId, {
        limit: Math.max(entries.length * 4, 50)
      });
      const existingKeys = new Set(
        existingEntries.entries.map((entry) => this.createMonologueKey(entry))
      );
      const newEntries = entries.filter((entry) => {
        return !existingKeys.has(this.createMonologueKey(entry));
      });

      await this.store.appendMonologue(canisterId, entries);

      const page = await this.store.listMonologue(canisterId, { limit: 50 });
      const canisterSnapshot = this.ensureCanisterSnapshot(canisterId);
      canisterSnapshot.lastIndexedMonologueCount = page.entries.length;
      canisterSnapshot.lastObservedTurnId = page.entries[0]?.turnId ?? null;

      for (const entry of newEntries.sort((left, right) => {
        if (left.timestamp === right.timestamp) {
          return left.turnId.localeCompare(right.turnId);
        }

        return left.timestamp - right.timestamp;
      })) {
        this.eventPublisher?.broadcast({ type: "monologue", canisterId, entry });
      }
    });
  }

  private async pollJournalFor(canisterId: string) {
    const readJournal = this.client.readJournal?.bind(this.client);
    if (!readJournal) return;
    await this.runPoll(canisterId, "journal", async () => {
      const response = await readJournal(canisterId);
      const entries: JournalEntry[] = response.entries.map((entry) => ({
        id: Number(entry.id),
        turnId: entry.turn_id,
        timestamp: Math.floor(Number(entry.timestamp_ns) / 1_000_000),
        text: entry.text,
        genesis: entry.genesis ?? false,
        dealClaim: entry.deal_claim ? {
          kind: entry.deal_claim.kind as "peer_payment_claim",
          version: entry.deal_claim.version as 1,
          txHash: entry.deal_claim.tx_hash,
          peerCanisterId: entry.deal_claim.peer_canister_id,
          asset: entry.deal_claim.asset as "eth" | "usdc",
          amountRaw: entry.deal_claim.amount_raw
        } : null
      }));
      const existing = await this.store.listJournal(canisterId, { limit: 200 });
      const existingIds = new Set(existing.entries.map((entry) => entry.id));
      const newEntries = entries.filter((entry) => !existingIds.has(entry.id));
      await this.store.appendJournal(canisterId, entries);

      const page = await this.store.listJournal(canisterId, { limit: 50 });
      this.ensureCanisterSnapshot(canisterId).lastIndexedJournalCount = page.entries.length;
      for (const entry of newEntries.sort((left, right) => left.id - right.id)) {
        this.eventPublisher?.broadcast({ type: "journal", canisterId, entry });
      }
    });
  }

  private async persistDetail(
    canisterId: string,
    previousDetail: AutomatonDetail | null | undefined,
    detail: AutomatonDetail
  ) {
    const changes = diffAutomatonRecord(previousDetail, detail);
    await this.store.upsertAutomaton(detail);

    const canisterSnapshot = this.ensureCanisterSnapshot(canisterId);
    canisterSnapshot.currentDetailAvailable = true;
    canisterSnapshot.lastPersistedAt = Date.now();

    if (changes) {
      this.eventPublisher?.broadcast({
        type: "update",
        canisterId,
        changes,
        timestamp: detail.lastPolledAt
      });
    }
  }

  private schedule(intervalMs: number, callback: () => Promise<void>) {
    const run = () => {
      void callback().catch(() => undefined);
    };

    run();
    const timer = setInterval(run, intervalMs);
    this.timers.add(timer);
  }

  private ensureCanisterSnapshot(canisterId: string) {
    this.snapshot.canisters[canisterId] ??= createCanisterPollSnapshot();
    return this.snapshot.canisters[canisterId];
  }

  private createMonologueKey(entry: MonologueEntry) {
    return `${entry.timestamp}:${entry.turnId}`;
  }

  private selectNewRoomMessages(messages: RoomMessage[], afterSeq: number | null) {
    return messages.filter((message) => {
      return afterSeq === null || message.seq > afterSeq;
    });
  }

  private async listTrackedCanisterIds() {
    return this.store.listTrackedCanisterIds();
  }

  private async readFactoryRegistry() {
    const records: SpawnedAutomatonRecord[] = [];
    let cursor: string | undefined;

    do {
      const page = await this.factoryClient!.listSpawnedAutomatons(cursor, 100);
      records.push(...page.items);
      cursor = page.nextCursor ?? undefined;
    } while (cursor !== undefined);

    return records;
  }

  private async runPoll(
    canisterId: string,
    phase: "identity" | "runtime" | "monologue" | "journal",
    operation: () => Promise<void>
  ) {
    const canisterSnapshot = this.ensureCanisterSnapshot(canisterId);
    const phaseSnapshot = canisterSnapshot[phase];

    if (phaseSnapshot.inFlight) {
      return;
    }

    phaseSnapshot.inFlight = true;
    phaseSnapshot.lastAttemptAt = Date.now();
    const startedAt = Date.now();

    try {
      await operation();
      phaseSnapshot.lastSuccessAt = Date.now();
      phaseSnapshot.lastError = null;
      phaseSnapshot.successCount += 1;
      phaseSnapshot.lastDurationMs = Date.now() - startedAt;
    } catch (error) {
      phaseSnapshot.lastError = error instanceof Error ? error.message : String(error);
      phaseSnapshot.failureCount += 1;
      phaseSnapshot.lastDurationMs = Date.now() - startedAt;
    } finally {
      phaseSnapshot.inFlight = false;
    }
  }
}
