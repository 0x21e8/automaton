PRAGMA journal_mode = WAL;
PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS automatons (
  canister_id TEXT PRIMARY KEY,
  steward_address TEXT NOT NULL,
  chain TEXT NOT NULL,
  tier TEXT NOT NULL,
  last_transition_at INTEGER NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  summary_json TEXT NOT NULL,
  detail_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS automatons_steward_idx
  ON automatons (steward_address);

CREATE INDEX IF NOT EXISTS automatons_chain_idx
  ON automatons (chain);

CREATE INDEX IF NOT EXISTS automatons_tier_idx
  ON automatons (tier);

CREATE TABLE IF NOT EXISTS monologue (
  canister_id TEXT NOT NULL,
  timestamp INTEGER NOT NULL,
  turn_id TEXT NOT NULL,
  entry_json TEXT NOT NULL,
  PRIMARY KEY (canister_id, timestamp, turn_id)
);

CREATE INDEX IF NOT EXISTS monologue_canister_ts_idx
  ON monologue (canister_id, timestamp DESC);

CREATE TABLE IF NOT EXISTS ens_cache (
  address TEXT PRIMARY KEY,
  ens_name TEXT,
  expires_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS prices (
  symbol TEXT PRIMARY KEY,
  value REAL,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS configured_canisters (
  canister_id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS configured_canisters_source_idx
  ON configured_canisters (source);

CREATE TABLE IF NOT EXISTS spawn_sessions (
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

CREATE INDEX IF NOT EXISTS spawn_sessions_state_idx
  ON spawn_sessions (state);

CREATE INDEX IF NOT EXISTS spawn_sessions_payment_status_idx
  ON spawn_sessions (payment_status);

CREATE INDEX IF NOT EXISTS spawn_sessions_claim_id_idx
  ON spawn_sessions (claim_id);

CREATE TABLE IF NOT EXISTS spawned_automaton_registry (
  canister_id TEXT PRIMARY KEY,
  session_id TEXT NOT NULL,
  steward_address TEXT NOT NULL,
  chain TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  updated_at INTEGER NOT NULL,
  record_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS spawned_automaton_registry_session_idx
  ON spawned_automaton_registry (session_id);

CREATE TABLE IF NOT EXISTS room_messages (
  seq INTEGER PRIMARY KEY,
  message_id TEXT NOT NULL UNIQUE,
  author_canister_id TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  content_type TEXT NOT NULL,
  body TEXT NOT NULL,
  mentions_json TEXT NOT NULL,
  message_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS room_messages_created_at_idx
  ON room_messages (created_at DESC, seq DESC);

CREATE INDEX IF NOT EXISTS room_messages_author_idx
  ON room_messages (author_canister_id, created_at DESC, seq DESC);

CREATE TABLE IF NOT EXISTS room_state (
  room_id TEXT PRIMARY KEY,
  latest_ingested_seq INTEGER,
  updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS faucet_claims (
  claim_id INTEGER PRIMARY KEY AUTOINCREMENT,
  wallet_address TEXT NOT NULL,
  ip_hash TEXT NOT NULL,
  claimed_at INTEGER NOT NULL,
  eth_amount TEXT NOT NULL,
  usdc_amount TEXT NOT NULL,
  tx_summary_json TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS faucet_claims_wallet_claimed_at_idx
  ON faucet_claims (wallet_address, claimed_at DESC);

CREATE INDEX IF NOT EXISTS faucet_claims_ip_claimed_at_idx
  ON faucet_claims (ip_hash, claimed_at DESC);
