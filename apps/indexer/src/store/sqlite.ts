import { access, mkdir, readFile } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname } from "node:path";

import type {
  AutomatonDetail,
  AutomatonListResponse,
  AutomatonSummary,
  AutomatonTier,
  ChainSlug,
  JournalEntry,
  JournalPage,
  MonologueEntry,
  MonologuePage,
  RoomMessage,
  RoomMessagePage,
  RoomMessageScope,
  SpawnSessionDetail,
  SpawnedAutomatonRecord
} from "@ic-automaton/shared";
import { verifyConstitution } from "../lib/genesis-integrity.js";

const require = createRequire(import.meta.url);

type SqliteValue = null | number | string;

interface BetterSqliteStatement<Row extends Record<string, SqliteValue>> {
  all(...parameters: SqliteValue[]): Row[];
  get(...parameters: SqliteValue[]): Row | undefined;
  run(...parameters: SqliteValue[]): void;
}

interface BetterSqliteDatabase {
  close(): void;
  exec(sql: string): this;
  prepare<Row extends Record<string, SqliteValue>>(sql: string): BetterSqliteStatement<Row>;
  transaction<TArgs extends unknown[]>(fn: (...args: TArgs) => void): (...args: TArgs) => void;
}

interface BetterSqliteConstructor {
  new (path: string): BetterSqliteDatabase;
}

const BetterSqlite3 = require("better-sqlite3") as BetterSqliteConstructor;

export interface AutomatonFilters {
  steward?: string;
  chain?: ChainSlug;
  tier?: AutomatonTier;
}

export interface MonologueQuery {
  before?: number;
  limit: number;
}

export interface JournalQuery {
  before?: number;
  limit: number;
}

export interface RoomMessageStoreQuery {
  afterSeq?: number;
  limit: number;
  canisterId?: string;
  scope?: RoomMessageScope;
}

export interface StoreHealth {
  ok: boolean;
  driver: "better-sqlite3";
  databasePath: string;
  counts: {
    configuredCanisters: number;
    trackedCanisters: number;
    automatons: number;
    monologueEntries: number;
    journalEntries: number;
    spawnSessions: number;
    spawnedAutomatonRegistryRecords: number;
  };
}

export interface SpawnRegistryQuery {
  cursor?: string;
  limit: number;
}

export interface FaucetClaimWindowQuery {
  walletAddress?: string;
  ipHash?: string;
  since: number;
}

export interface FaucetClaimWindowStats {
  count: number;
  oldestClaimAt: number | null;
}

export interface FaucetClaimRecord {
  walletAddress: string;
  ipHash: string;
  claimedAt: number;
  ethAmount: string;
  usdcAmount: string;
  txSummary: Record<string, unknown>;
}

export interface IndexerStore {
  initialize(): Promise<void>;
  close(): Promise<void>;
  getHealth(): Promise<StoreHealth>;
  listConfiguredCanisterIds(): Promise<string[]>;
  listFactoryDiscoveredCanisterIds(): Promise<string[]>;
  listTrackedCanisterIds(): Promise<string[]>;
  syncConfiguredCanisterIds(canisterIds: string[]): Promise<void>;
  listAutomatons(filters?: AutomatonFilters): Promise<AutomatonListResponse>;
  getAutomatonDetail(canisterId: string): Promise<AutomatonDetail | null>;
  upsertAutomaton(detail: AutomatonDetail): Promise<void>;
  listMonologue(canisterId: string, query: MonologueQuery): Promise<MonologuePage>;
  appendMonologue(canisterId: string, entries: MonologueEntry[]): Promise<void>;
  listJournal(canisterId: string, query: JournalQuery): Promise<JournalPage>;
  appendJournal(canisterId: string, entries: JournalEntry[]): Promise<void>;
  getLatestRoomMessageSeq(): Promise<number | null>;
  listRoomMessages(query: RoomMessageStoreQuery): Promise<RoomMessagePage>;
  upsertRoomMessages(messages: RoomMessage[], latestSeq: number | null): Promise<void>;
  pruneRoomMessages(olderThan: number): Promise<void>;
  listSpawnSessionDetails(limit: number): Promise<SpawnSessionDetail[]>;
  getSpawnSessionDetail(sessionId: string): Promise<SpawnSessionDetail | null>;
  upsertSpawnSession(detail: SpawnSessionDetail): Promise<void>;
  listSpawnedAutomatonRegistry(query: SpawnRegistryQuery): Promise<{
    items: SpawnedAutomatonRecord[];
    nextCursor: string | null;
  }>;
  getSpawnedAutomatonRegistryRecord(canisterId: string): Promise<SpawnedAutomatonRecord | null>;
  upsertSpawnedAutomatonRegistry(records: SpawnedAutomatonRecord[]): Promise<void>;
  replaceSpawnedAutomatonRegistry(records: SpawnedAutomatonRecord[]): Promise<void>;
  getFaucetClaimWindowStats(query: FaucetClaimWindowQuery): Promise<FaucetClaimWindowStats>;
  recordFaucetClaim(claim: FaucetClaimRecord): Promise<void>;
  setPrice(symbol: string, value: number | null): Promise<void>;
}

interface SqliteStoreOptions {
  databasePath: string;
}

function detailToSummary(detail: AutomatonDetail): AutomatonSummary {
  return {
    canisterId: detail.canisterId,
    ethAddress: detail.ethAddress,
    chain: detail.chain,
    chainId: detail.chainId,
    name: detail.name,
    constitutionHash: detail.constitutionHash,
    tier: detail.tier,
    agentState: detail.runtime.agentState,
    ethBalanceWei: detail.financials.ethBalanceWei,
    usdcBalanceRaw: detail.financials.usdcBalanceRaw,
    cyclesBalance: detail.financials.cyclesBalance,
    metabolism: detail.metabolism,
    controlStatus: detail.controlStatus,
    netWorthEth: detail.financials.netWorthEth,
    netWorthUsd: detail.financials.netWorthUsd,
    heartbeatIntervalSeconds: detail.runtime.heartbeatIntervalSeconds,
    steward: detail.steward,
    gridPosition: detail.gridPosition,
    corePatternIndex: detail.corePatternIndex,
    corePattern: detail.corePattern,
    parentId: detail.parentId,
    createdAt: detail.createdAt,
    lastTransitionAt: detail.runtime.lastTransitionAt
  };
}

async function loadSchemaSql() {
  const candidates = [
    new URL("./schema.sql", import.meta.url),
    new URL("../../src/store/schema.sql", import.meta.url)
  ];

  for (const candidate of candidates) {
    try {
      await access(candidate);
      return await readFile(candidate, "utf8");
    } catch {
      continue;
    }
  }

  throw new Error("Unable to locate SQLite schema.sql");
}

function tableHasColumn(
  database: BetterSqliteDatabase,
  tableName: string,
  columnName: string
) {
  const rows = database
    .prepare<{ name: string }>(`PRAGMA table_info(${tableName});`)
    .all();

  return rows.some((row) => row.name === columnName);
}

function tableExists(database: BetterSqliteDatabase, tableName: string) {
  const row = database
    .prepare<{ name: string }>(
      "SELECT name FROM sqlite_master WHERE type = 'table' AND name = ? LIMIT 1;"
    )
    .get(tableName);

  return row !== undefined;
}

function migrateSpawnSessionsSchema(database: BetterSqliteDatabase) {
  const hasEscrowJson = tableHasColumn(database, "spawn_sessions", "escrow_json");
  const hasClaimId = tableHasColumn(database, "spawn_sessions", "claim_id");
  const hasPaymentJson = tableHasColumn(database, "spawn_sessions", "payment_json");
  const hasReleaseTxHash = tableHasColumn(database, "spawn_sessions", "release_tx_hash");
  const hasReleaseBroadcastAt = tableHasColumn(
    database,
    "spawn_sessions",
    "release_broadcast_at"
  );

  if (
    !hasEscrowJson &&
    hasClaimId &&
    hasPaymentJson &&
    hasReleaseTxHash &&
    hasReleaseBroadcastAt
  ) {
    return;
  }

  const claimIdExpression = hasClaimId
    ? "COALESCE(claim_id, json_extract(session_json, '$.claimId'))"
    : "json_extract(session_json, '$.claimId')";
  const paymentJsonExpression = hasPaymentJson ? "payment_json" : "NULL";
  const releaseTxHashExpression = hasReleaseTxHash
    ? "COALESCE(release_tx_hash, json_extract(session_json, '$.releaseTxHash'))"
    : "json_extract(session_json, '$.releaseTxHash')";
  const releaseBroadcastAtExpression = hasReleaseBroadcastAt
    ? "COALESCE(release_broadcast_at, json_extract(session_json, '$.releaseBroadcastAt'))"
    : "json_extract(session_json, '$.releaseBroadcastAt')";

  database.exec(`
    ALTER TABLE spawn_sessions RENAME TO spawn_sessions_legacy;

    CREATE TABLE spawn_sessions (
      session_id TEXT PRIMARY KEY,
      state TEXT NOT NULL,
      payment_status TEXT NOT NULL,
      retryable INTEGER NOT NULL,
      refundable INTEGER NOT NULL,
      claim_id TEXT NOT NULL,
      release_tx_hash TEXT,
      release_broadcast_at INTEGER,
      updated_at INTEGER NOT NULL,
      session_json TEXT NOT NULL,
      payment_json TEXT,
      audit_json TEXT NOT NULL,
      registry_json TEXT
    );

    INSERT INTO spawn_sessions (
      session_id,
      state,
      payment_status,
      retryable,
      refundable,
      claim_id,
      release_tx_hash,
      release_broadcast_at,
      updated_at,
      session_json,
      payment_json,
      audit_json,
      registry_json
    )
    SELECT
      session_id,
      state,
      payment_status,
      retryable,
      refundable,
      ${claimIdExpression},
      ${releaseTxHashExpression},
      ${releaseBroadcastAtExpression},
      updated_at,
      session_json,
      ${paymentJsonExpression},
      audit_json,
      registry_json
    FROM spawn_sessions_legacy;

    DROP TABLE spawn_sessions_legacy;
  `);
}

function normalizeRegistryRecords(records: SpawnedAutomatonRecord[]) {
  return [...new Map(records.map((record) => [record.canisterId, record])).values()].sort(
    (left, right) => left.canisterId.localeCompare(right.canisterId)
  );
}

const SHARED_ROOM_ID = "shared";

class BetterSqliteStore implements IndexerStore {
  readonly databasePath: string;

  private database?: BetterSqliteDatabase;
  private initialized = false;
  private readonly schemaSqlPromise: Promise<string>;

  constructor(options: SqliteStoreOptions) {
    this.databasePath = options.databasePath;
    this.schemaSqlPromise = loadSchemaSql();
  }

  async initialize() {
    if (this.initialized) {
      return;
    }

    await mkdir(dirname(this.databasePath), { recursive: true });
    this.database = new BetterSqlite3(this.databasePath);
    const schemaSql = await this.schemaSqlPromise;

    if (tableExists(this.database, "spawn_sessions")) {
      migrateSpawnSessionsSchema(this.database);
    }

    this.database.exec(schemaSql);
    this.initialized = true;
  }

  async close() {
    this.database?.close();
    this.database = undefined;
    this.initialized = false;
  }

  async getHealth(): Promise<StoreHealth> {
    await this.initialize();
    const database = this.getDatabase();
    const configuredCanisterCountRow = database
      .prepare<{ count: number }>("SELECT COUNT(*) AS count FROM configured_canisters;")
      .get();
    const automatonCountRow = database
      .prepare<{ count: number }>("SELECT COUNT(*) AS count FROM automatons;")
      .get();
    const monologueCountRow = database
      .prepare<{ count: number }>("SELECT COUNT(*) AS count FROM monologue;")
      .get();
    const journalCountRow = database
      .prepare<{ count: number }>("SELECT COUNT(*) AS count FROM journal_entries;")
      .get();
    const spawnSessionCountRow = database
      .prepare<{ count: number }>("SELECT COUNT(*) AS count FROM spawn_sessions;")
      .get();
    const spawnRegistryCountRow = database
      .prepare<{ count: number }>("SELECT COUNT(*) AS count FROM spawned_automaton_registry;")
      .get();
    const trackedCanisterCountRow = database
      .prepare<{ count: number }>(
        `SELECT COUNT(*) AS count
         FROM (
           SELECT canister_id FROM configured_canisters
           UNION
           SELECT canister_id FROM spawned_automaton_registry
         );`
      )
      .get();

    return {
      ok: true,
      driver: "better-sqlite3",
      databasePath: this.databasePath,
      counts: {
        configuredCanisters: Number(configuredCanisterCountRow?.count ?? 0),
        trackedCanisters: Number(trackedCanisterCountRow?.count ?? 0),
        automatons: Number(automatonCountRow?.count ?? 0),
        monologueEntries: Number(monologueCountRow?.count ?? 0),
        journalEntries: Number(journalCountRow?.count ?? 0),
        spawnSessions: Number(spawnSessionCountRow?.count ?? 0),
        spawnedAutomatonRegistryRecords: Number(spawnRegistryCountRow?.count ?? 0)
      }
    };
  }

  async listConfiguredCanisterIds() {
    await this.initialize();
    const database = this.getDatabase();
    const rows = database
      .prepare<{ canister_id: string }>(
        `SELECT canister_id
         FROM configured_canisters
         ORDER BY canister_id ASC;`
      )
      .all();

    return rows.map((row) => row.canister_id);
  }

  async listFactoryDiscoveredCanisterIds() {
    await this.initialize();
    const database = this.getDatabase();
    const rows = database
      .prepare<{ canister_id: string }>(
        `SELECT canister_id
         FROM spawned_automaton_registry
         ORDER BY canister_id ASC;`
      )
      .all();

    return rows.map((row) => row.canister_id);
  }

  async listTrackedCanisterIds() {
    await this.initialize();
    const database = this.getDatabase();
    const rows = database
      .prepare<{ canister_id: string }>(
        `SELECT canister_id
         FROM configured_canisters
         UNION
         SELECT canister_id
         FROM spawned_automaton_registry
         ORDER BY canister_id ASC;`
      )
      .all();

    return rows.map((row) => row.canister_id);
  }

  async syncConfiguredCanisterIds(canisterIds: string[]) {
    await this.initialize();
    const database = this.getDatabase();
    const normalizedCanisterIds = [...new Set(canisterIds.map((canisterId) => canisterId.trim()))];
    const updatedAt = Date.now();
    const upsertStatement = database.prepare(
      `INSERT INTO configured_canisters (canister_id, source, updated_at)
       VALUES (?, 'config', ?)
       ON CONFLICT(canister_id) DO UPDATE SET
         source = excluded.source,
         updated_at = excluded.updated_at;`
    );
    const deleteMissingStatement = database.prepare(
      `DELETE FROM configured_canisters
       WHERE source = 'config'
         AND canister_id NOT IN (${normalizedCanisterIds.map(() => "?").join(", ")});`
    );
    const syncConfiguredCanisters = database.transaction((configuredIds: string[]) => {
      for (const canisterId of configuredIds) {
        upsertStatement.run(canisterId, updatedAt);
      }

      deleteMissingStatement.run(...configuredIds);
    });

    syncConfiguredCanisters(normalizedCanisterIds);
  }

  async listAutomatons(filters: AutomatonFilters = {}): Promise<AutomatonListResponse> {
    await this.initialize();
    const database = this.getDatabase();

    const where: string[] = [];
    const parameters: SqliteValue[] = [];

    if (filters.steward) {
      where.push("steward_address = ?");
      parameters.push(filters.steward);
    }
    if (filters.chain) {
      where.push("chain = ?");
      parameters.push(filters.chain);
    }
    if (filters.tier) {
      where.push("tier = ?");
      parameters.push(filters.tier);
    }

    const whereClause = where.length > 0 ? `WHERE ${where.join(" AND ")}` : "";
    const summaryRows = database
      .prepare<{ summary_json: string }>(
        `SELECT summary_json
         FROM automatons
         ${whereClause}
         ORDER BY last_transition_at DESC, canister_id ASC;`
      )
      .all(...parameters);
    const summaries = summaryRows.map((row) => {
      return JSON.parse(row.summary_json) as AutomatonSummary;
    });
    const price = await this.getPrice("ethUsd");

    return {
      automatons: summaries,
      total: summaries.length,
      prices: {
        ethUsd: price
      }
    };
  }

  async getAutomatonDetail(canisterId: string) {
    await this.initialize();
    const database = this.getDatabase();
    const row = database
      .prepare<{ detail_json: string }>(
        `SELECT detail_json
         FROM automatons
         WHERE canister_id = ?
         LIMIT 1;`
      )
      .get(canisterId);

    if (!row) {
      return null;
    }

    const detail = JSON.parse(row.detail_json) as AutomatonDetail;
    const constitutionResult =
      detail.constitutionVerification === undefined
        ? verifyConstitution(
            detail.constitution ?? null,
            detail.constitutionHash ?? null
          )
        : {
            constitution: detail.constitution ?? null,
            verification: detail.constitutionVerification
          };
    const monologue = await this.listMonologue(canisterId, {
      limit: 50
    });
    const journal = await this.listJournal(canisterId, { limit: 50 });

    return {
      ...detail,
      constitution: constitutionResult.constitution,
      constitutionVerification: constitutionResult.verification,
      monologue: monologue.entries,
      journal: journal.entries
    };
  }

  async upsertAutomaton(detail: AutomatonDetail) {
    await this.initialize();
    const database = this.getDatabase();
    const summary = detailToSummary(detail);
    const now = Date.now();

    database
      .prepare(
        `INSERT INTO automatons (
          canister_id,
          steward_address,
          chain,
          tier,
          last_transition_at,
          created_at,
          updated_at,
          summary_json,
          detail_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(canister_id) DO UPDATE SET
          steward_address = excluded.steward_address,
          chain = excluded.chain,
          tier = excluded.tier,
          last_transition_at = excluded.last_transition_at,
          updated_at = excluded.updated_at,
          summary_json = excluded.summary_json,
          detail_json = excluded.detail_json;`
      )
      .run(
        detail.canisterId,
        detail.steward.address,
        detail.chain,
        detail.tier,
        detail.runtime.lastTransitionAt,
        detail.createdAt,
        now,
        JSON.stringify(summary),
        JSON.stringify(detail)
      );
  }

  async listMonologue(canisterId: string, query: MonologueQuery): Promise<MonologuePage> {
    await this.initialize();
    const database = this.getDatabase();

    const clauses = ["canister_id = ?"];
    const parameters: SqliteValue[] = [canisterId];
    if (query.before !== undefined) {
      clauses.push("timestamp < ?");
      parameters.push(query.before);
    }
    parameters.push(query.limit + 1);

    const rows = database
      .prepare<{ entry_json: string }>(
        `SELECT entry_json
         FROM monologue
         WHERE ${clauses.join(" AND ")}
         ORDER BY timestamp DESC, turn_id DESC
         LIMIT ?;`
      )
      .all(...parameters);

    const hasMore = rows.length > query.limit;
    const entries = rows.slice(0, query.limit).map((row) => {
      return JSON.parse(row.entry_json) as MonologueEntry;
    });
    const lastEntry = entries.at(-1);

    return {
      entries,
      hasMore,
      nextCursor: hasMore && lastEntry ? lastEntry.timestamp : null
    };
  }

  async appendMonologue(canisterId: string, entries: MonologueEntry[]) {
    await this.initialize();
    const database = this.getDatabase();

    if (entries.length === 0) {
      return;
    }

    const statement = database.prepare(
      `INSERT INTO monologue (canister_id, timestamp, turn_id, entry_json)
       VALUES (?, ?, ?, ?)
       ON CONFLICT(canister_id, timestamp, turn_id) DO UPDATE SET
         entry_json = excluded.entry_json;`
    );
    const insertEntries = database.transaction((records: MonologueEntry[]) => {
      for (const entry of records) {
        statement.run(canisterId, entry.timestamp, entry.turnId, JSON.stringify(entry));
      }
    });

    insertEntries(entries);
  }

  async listJournal(canisterId: string, query: JournalQuery): Promise<JournalPage> {
    await this.initialize();
    const database = this.getDatabase();
    const clauses = ["canister_id = ?"];
    const parameters: SqliteValue[] = [canisterId];
    if (query.before !== undefined) {
      clauses.push("entry_id < ?");
      parameters.push(query.before);
    }
    parameters.push(query.limit + 1);
    const rows = database
      .prepare<{ entry_json: string }>(
        `SELECT entry_json FROM journal_entries
         WHERE ${clauses.join(" AND ")}
         ORDER BY entry_id DESC LIMIT ?;`
      )
      .all(...parameters);
    const hasMore = rows.length > query.limit;
    const entries = rows.slice(0, query.limit).map((row) => JSON.parse(row.entry_json) as JournalEntry);
    return {
      entries,
      hasMore,
      nextCursor: hasMore ? entries.at(-1)?.id ?? null : null
    };
  }

  async appendJournal(canisterId: string, entries: JournalEntry[]) {
    await this.initialize();
    if (entries.length === 0) return;
    const database = this.getDatabase();
    const statement = database.prepare(
      `INSERT INTO journal_entries (canister_id, entry_id, timestamp, turn_id, entry_json)
       VALUES (?, ?, ?, ?, ?)
       ON CONFLICT(canister_id, entry_id) DO UPDATE SET
         timestamp = excluded.timestamp,
         turn_id = excluded.turn_id,
         entry_json = excluded.entry_json;`
    );
    database.transaction((records: JournalEntry[]) => {
      for (const entry of records) {
        statement.run(canisterId, entry.id, entry.timestamp, entry.turnId, JSON.stringify(entry));
      }
    })(entries);
  }

  async getLatestRoomMessageSeq() {
    await this.initialize();
    const database = this.getDatabase();
    const stateRow = database
      .prepare<{ latest_ingested_seq: number | null }>(
        `SELECT latest_ingested_seq
         FROM room_state
         WHERE room_id = ?
         LIMIT 1;`
      )
      .get(SHARED_ROOM_ID);

    if (stateRow?.latest_ingested_seq !== null && stateRow?.latest_ingested_seq !== undefined) {
      return Number(stateRow.latest_ingested_seq);
    }

    const messageRow = database
      .prepare<{ latest_seq: number | null }>(
        `SELECT MAX(seq) AS latest_seq
         FROM room_messages;`
      )
      .get();

    return messageRow?.latest_seq === null || messageRow?.latest_seq === undefined
      ? null
      : Number(messageRow.latest_seq);
  }

  async listRoomMessages(query: RoomMessageStoreQuery): Promise<RoomMessagePage> {
    await this.initialize();
    const database = this.getDatabase();
    const clauses: string[] = [];
    const parameters: SqliteValue[] = [];

    if (query.afterSeq !== undefined) {
      clauses.push("seq > ?");
      parameters.push(query.afterSeq);
    }

    if (query.scope === "relevant" && query.canisterId) {
      clauses.push(`(
        json_array_length(mentions_json) = 0
        OR EXISTS (
          SELECT 1
          FROM json_each(room_messages.mentions_json)
          WHERE json_each.value = ?
        )
      )`);
      parameters.push(query.canisterId);
    }

    parameters.push(query.limit + 1);
    const whereClause = clauses.length > 0 ? `WHERE ${clauses.join(" AND ")}` : "";
    const rows = database
      .prepare<{ message_json: string }>(
        `SELECT message_json
         FROM room_messages
         ${whereClause}
         ORDER BY seq ASC
         LIMIT ?;`
      )
      .all(...parameters);

    const hasMore = rows.length > query.limit;
    const messages = rows.slice(0, query.limit).map((row) => {
      return JSON.parse(row.message_json) as RoomMessage;
    });
    const lastMessage = messages.at(-1);

    return {
      messages,
      nextAfterSeq: hasMore && lastMessage ? lastMessage.seq : null,
      latestSeq: await this.getLatestRoomMessageSeq()
    };
  }

  async upsertRoomMessages(messages: RoomMessage[], latestSeq: number | null) {
    await this.initialize();
    const database = this.getDatabase();
    const effectiveLatestSeq =
      latestSeq ?? messages.reduce<number | null>((current, message) => {
        return current === null || message.seq > current ? message.seq : current;
      }, null);

    const insertMessage = database.prepare(
      `INSERT INTO room_messages (
        seq,
        message_id,
        author_canister_id,
        created_at,
        content_type,
        body,
        mentions_json,
        message_json
      ) VALUES (?, ?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(seq) DO UPDATE SET
        message_id = excluded.message_id,
        author_canister_id = excluded.author_canister_id,
        created_at = excluded.created_at,
        content_type = excluded.content_type,
        body = excluded.body,
        mentions_json = excluded.mentions_json,
        message_json = excluded.message_json;`
    );
    const upsertRoomState = database.prepare(
      `INSERT INTO room_state (
        room_id,
        latest_ingested_seq,
        updated_at
      ) VALUES (?, ?, ?)
      ON CONFLICT(room_id) DO UPDATE SET
        latest_ingested_seq = MAX(
          COALESCE(room_state.latest_ingested_seq, excluded.latest_ingested_seq),
          excluded.latest_ingested_seq
        ),
        updated_at = excluded.updated_at;`
    );
    const persistMessages = database.transaction((roomMessages: RoomMessage[]) => {
      for (const message of roomMessages) {
        insertMessage.run(
          message.seq,
          message.messageId,
          message.authorCanisterId,
          message.createdAt,
          message.contentType,
          message.body,
          JSON.stringify(message.mentions),
          JSON.stringify(message)
        );
      }

      if (effectiveLatestSeq !== null) {
        upsertRoomState.run(SHARED_ROOM_ID, effectiveLatestSeq, Date.now());
      }
    });

    persistMessages(messages);
  }

  async pruneRoomMessages(olderThan: number) {
    await this.initialize();
    const database = this.getDatabase();

    database
      .prepare(
        `DELETE FROM room_messages
         WHERE created_at < ?;`
      )
      .run(olderThan);
  }

  async getSpawnSessionDetail(sessionId: string) {
    await this.initialize();
    const database = this.getDatabase();
    const row = database
      .prepare<{
        session_json: string;
        payment_json: string | null;
        audit_json: string;
        registry_json: string | null;
      }>(
        `SELECT session_json, payment_json, audit_json, registry_json
         FROM spawn_sessions
         WHERE session_id = ?
         LIMIT 1;`
      )
      .get(sessionId);

    if (!row) {
      return null;
    }

    if (row.payment_json === null) {
      return null;
    }

    return {
      session: JSON.parse(row.session_json) as SpawnSessionDetail["session"],
      payment: JSON.parse(row.payment_json) as SpawnSessionDetail["payment"],
      audit: JSON.parse(row.audit_json) as SpawnSessionDetail["audit"],
      registryRecord:
        row.registry_json === null
          ? null
          : (JSON.parse(row.registry_json) as SpawnSessionDetail["registryRecord"])
    } satisfies SpawnSessionDetail;
  }

  async listSpawnSessionDetails(limit: number) {
    await this.initialize();
    const database = this.getDatabase();
    const rows = database
      .prepare<{
        session_json: string;
        payment_json: string | null;
        audit_json: string;
        registry_json: string | null;
      }>(
        `SELECT session_json, payment_json, audit_json, registry_json
         FROM spawn_sessions
         WHERE payment_json IS NOT NULL
         ORDER BY updated_at DESC, session_id DESC
         LIMIT ?;`
      )
      .all(limit);

    return rows.map((row) => {
      return {
        session: JSON.parse(row.session_json) as SpawnSessionDetail["session"],
        payment: JSON.parse(row.payment_json ?? "null") as SpawnSessionDetail["payment"],
        audit: JSON.parse(row.audit_json) as SpawnSessionDetail["audit"],
        registryRecord:
          row.registry_json === null
            ? null
            : (JSON.parse(row.registry_json) as SpawnSessionDetail["registryRecord"])
      } satisfies SpawnSessionDetail;
    });
  }

  async upsertSpawnSession(detail: SpawnSessionDetail) {
    await this.initialize();
    const database = this.getDatabase();

    database
      .prepare(
        `INSERT INTO spawn_sessions (
          session_id,
          state,
          payment_status,
          retryable,
          refundable,
          claim_id,
          release_tx_hash,
          release_broadcast_at,
          updated_at,
          session_json,
          payment_json,
          audit_json,
          registry_json
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(session_id) DO UPDATE SET
          state = excluded.state,
          payment_status = excluded.payment_status,
          retryable = excluded.retryable,
          refundable = excluded.refundable,
          claim_id = excluded.claim_id,
          release_tx_hash = excluded.release_tx_hash,
          release_broadcast_at = excluded.release_broadcast_at,
          updated_at = excluded.updated_at,
          session_json = excluded.session_json,
          payment_json = excluded.payment_json,
          audit_json = excluded.audit_json,
          registry_json = excluded.registry_json;`
      )
      .run(
        detail.session.sessionId,
        detail.session.state,
        detail.session.paymentStatus,
        detail.session.retryable ? 1 : 0,
        detail.session.refundable ? 1 : 0,
        detail.session.claimId,
        detail.session.releaseTxHash,
        detail.session.releaseBroadcastAt,
        detail.session.updatedAt,
        JSON.stringify(detail.session),
        JSON.stringify(detail.payment),
        JSON.stringify(detail.audit),
        detail.registryRecord ? JSON.stringify(detail.registryRecord) : null
      );

    if (detail.registryRecord) {
      await this.upsertSpawnedAutomatonRegistry([detail.registryRecord]);
    }
  }

  async listSpawnedAutomatonRegistry(query: SpawnRegistryQuery) {
    await this.initialize();
    const database = this.getDatabase();
    const clauses: string[] = [];
    const parameters: SqliteValue[] = [];

    if (query.cursor) {
      clauses.push("canister_id > ?");
      parameters.push(query.cursor);
    }

    parameters.push(query.limit + 1);
    const whereClause = clauses.length > 0 ? `WHERE ${clauses.join(" AND ")}` : "";
    const rows = database
      .prepare<{ record_json: string }>(
        `SELECT record_json
         FROM spawned_automaton_registry
         ${whereClause}
         ORDER BY canister_id ASC
         LIMIT ?;`
      )
      .all(...parameters);

    const hasMore = rows.length > query.limit;
    const items = rows.slice(0, query.limit).map((row) => {
      return JSON.parse(row.record_json) as SpawnedAutomatonRecord;
    });

    return {
      items,
      nextCursor: hasMore ? items.at(-1)?.canisterId ?? null : null
    };
  }

  async getSpawnedAutomatonRegistryRecord(canisterId: string) {
    await this.initialize();
    const database = this.getDatabase();
    const row = database
      .prepare<{ record_json: string }>(
        `SELECT record_json
         FROM spawned_automaton_registry
         WHERE canister_id = ?
         LIMIT 1;`
      )
      .get(canisterId);

    if (!row) {
      return null;
    }

    return JSON.parse(row.record_json) as SpawnedAutomatonRecord;
  }

  async upsertSpawnedAutomatonRegistry(records: SpawnedAutomatonRecord[]) {
    await this.initialize();
    const database = this.getDatabase();
    const normalizedRecords = normalizeRegistryRecords(records);

    if (normalizedRecords.length === 0) {
      return;
    }

    const statement = database.prepare(
      `INSERT INTO spawned_automaton_registry (
        canister_id,
        session_id,
        steward_address,
        chain,
        created_at,
        updated_at,
        record_json
      ) VALUES (?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(canister_id) DO UPDATE SET
        session_id = excluded.session_id,
        steward_address = excluded.steward_address,
        chain = excluded.chain,
        created_at = excluded.created_at,
        updated_at = excluded.updated_at,
        record_json = excluded.record_json;`
    );
    const upsertRecords = database.transaction((registryRecords: SpawnedAutomatonRecord[]) => {
      const updatedAt = Date.now();

      for (const record of registryRecords) {
        statement.run(
          record.canisterId,
          record.sessionId,
          record.stewardAddress,
          record.chain,
          record.createdAt,
          updatedAt,
          JSON.stringify(record)
        );
      }
    });

    upsertRecords(normalizedRecords);
  }

  async replaceSpawnedAutomatonRegistry(records: SpawnedAutomatonRecord[]) {
    await this.initialize();
    const database = this.getDatabase();
    const normalizedRecords = normalizeRegistryRecords(records);
    const updatedAt = Date.now();
    const upsertStatement = database.prepare(
      `INSERT INTO spawned_automaton_registry (
        canister_id,
        session_id,
        steward_address,
        chain,
        created_at,
        updated_at,
        record_json
      ) VALUES (?, ?, ?, ?, ?, ?, ?)
      ON CONFLICT(canister_id) DO UPDATE SET
        session_id = excluded.session_id,
        steward_address = excluded.steward_address,
        chain = excluded.chain,
        created_at = excluded.created_at,
        updated_at = excluded.updated_at,
        record_json = excluded.record_json;`
    );
    const deleteAllStatement = database.prepare("DELETE FROM spawned_automaton_registry;");
    const deleteMissingStatement =
      normalizedRecords.length > 0
        ? database.prepare(
            `DELETE FROM spawned_automaton_registry
             WHERE canister_id NOT IN (${normalizedRecords.map(() => "?").join(", ")});`
          )
        : null;
    const replaceRecords = database.transaction((registryRecords: SpawnedAutomatonRecord[]) => {
      if (registryRecords.length === 0) {
        deleteAllStatement.run();
        return;
      }

      for (const record of registryRecords) {
        upsertStatement.run(
          record.canisterId,
          record.sessionId,
          record.stewardAddress,
          record.chain,
          record.createdAt,
          updatedAt,
          JSON.stringify(record)
        );
      }

      deleteMissingStatement?.run(...registryRecords.map((record) => record.canisterId));
    });

    replaceRecords(normalizedRecords);
  }

  async getFaucetClaimWindowStats(query: FaucetClaimWindowQuery) {
    await this.initialize();
    const database = this.getDatabase();
    const clauses = ["claimed_at >= ?"];
    const parameters: SqliteValue[] = [query.since];

    if (query.walletAddress) {
      clauses.push("wallet_address = ?");
      parameters.push(query.walletAddress);
    }

    if (query.ipHash) {
      clauses.push("ip_hash = ?");
      parameters.push(query.ipHash);
    }

    if (clauses.length === 1) {
      throw new Error("Faucet claim stats query requires a wallet address or IP hash.");
    }

    const row = database
      .prepare<{
        count: number;
        oldest_claim_at: number | null;
      }>(
        `SELECT COUNT(*) AS count, MIN(claimed_at) AS oldest_claim_at
         FROM faucet_claims
         WHERE ${clauses.join(" AND ")};`
      )
      .get(...parameters);

    return {
      count: Number(row?.count ?? 0),
      oldestClaimAt:
        row?.oldest_claim_at === null || row?.oldest_claim_at === undefined
          ? null
          : Number(row.oldest_claim_at)
    };
  }

  async recordFaucetClaim(claim: FaucetClaimRecord) {
    await this.initialize();
    const database = this.getDatabase();

    database
      .prepare(
        `INSERT INTO faucet_claims (
          wallet_address,
          ip_hash,
          claimed_at,
          eth_amount,
          usdc_amount,
          tx_summary_json
        ) VALUES (?, ?, ?, ?, ?, ?);`
      )
      .run(
        claim.walletAddress,
        claim.ipHash,
        claim.claimedAt,
        claim.ethAmount,
        claim.usdcAmount,
        JSON.stringify(claim.txSummary)
      );
  }

  async setPrice(symbol: string, value: number | null) {
    await this.initialize();
    const database = this.getDatabase();

    database
      .prepare(
        `INSERT INTO prices (symbol, value, updated_at)
         VALUES (?, ?, ?)
         ON CONFLICT(symbol) DO UPDATE SET
           value = excluded.value,
           updated_at = excluded.updated_at;`
      )
      .run(symbol, value, Date.now());
  }

  private async getPrice(symbol: string) {
    const database = this.getDatabase();
    const row = database
      .prepare<{ value: number | null }>(
        `SELECT value
         FROM prices
         WHERE symbol = ?
         LIMIT 1;`
      )
      .get(symbol);

    return row?.value === null || row?.value === undefined ? null : Number(row.value);
  }

  private getDatabase() {
    if (!this.database) {
      throw new Error("SQLite store accessed before initialization");
    }

    return this.database;
  }
}

export function createSqliteStore(options: SqliteStoreOptions): IndexerStore {
  return new BetterSqliteStore(options);
}
