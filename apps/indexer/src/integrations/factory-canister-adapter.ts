import {
  Actor,
  HttpAgent,
  type ActorMethod,
  type ActorSubclass
} from "@dfinity/agent";
import { IDL } from "@dfinity/candid";
import type {
  CreateSpawnSessionRequest,
  CreateSpawnSessionResponse,
  FactoryStewardCommand,
  FactoryStewardExecutionRequest,
  FactoryStewardProofTemplate,
  PaymentStatus,
  RepositoryStrategyGetResponse,
  RepositoryStrategyListResponse,
  RepositoryStrategyRecord,
  RefundSpawnResponse,
  RoomContentType,
  RoomMessagePage,
  RetrySpawnResponse,
  SessionAuditActor,
  SpawnAsset,
  SpawnChain,
  SpawnSessionState,
  SpawnSessionStatusResponse,
  SpawnedAutomatonRecord,
  SpawnedAutomatonRegistryPage
} from "@ic-automaton/shared";

import type { FactoryAdapter, FactoryHealthSnapshot } from "./factory-client.js";

type Optional<T> = [] | [T];

type CandidVariant<TName extends string, TValue = null> = {
  [Name in TName]?: TValue;
};

type CandidSpawnChain = CandidVariant<"Base">;
type CandidSpawnAsset = CandidVariant<"Usdc">;
type CandidInferenceTransport = CandidVariant<
  "OpenrouterDirect" | "OpenrouterProxyWorker"
>;
type CandidOpenRouterReasoningLevel = CandidVariant<
  "Default" | "Low" | "Medium" | "High"
>;
type CandidSpawnSessionState = CandidVariant<
  | "AwaitingPayment"
  | "PaymentDetected"
  | "Spawning"
  | "BroadcastingRelease"
  | "Complete"
  | "Failed"
  | "Expired"
>;
type CandidPaymentStatus = CandidVariant<"Unpaid" | "Partial" | "Paid" | "Refunded">;
type CandidSessionAuditActor = CandidVariant<"System" | "User" | "Admin">;
type CandidRepositoryStrategyStatus = CandidVariant<"Active" | "Deprecated" | "Revoked">;

interface CandidProviderConfig {
  inference_transport: CandidInferenceTransport;
  model: Optional<string>;
  open_router_reasoning_level: CandidOpenRouterReasoningLevel;
}

interface CandidSpawnProviderSecrets {
  brave_search_api_key: Optional<string>;
  open_router_api_key: Optional<string>;
}

interface CandidSpawnConfig {
  chain: CandidSpawnChain;
  provider: CandidProviderConfig;
  risk: number;
  skills: string[];
  strategies: string[];
}

interface CandidCreateSpawnSessionRequest {
  name: Optional<string>;
  constitution: Optional<string>;
  asset: CandidSpawnAsset;
  config: CandidSpawnConfig;
  gross_amount: string;
  parent_id: Optional<string>;
  provider_secrets: CandidSpawnProviderSecrets;
  steward_address: string;
}

interface CandidSpawnPaymentInstructions {
  asset: CandidSpawnAsset;
  chain: CandidSpawnChain;
  claim_id: string;
  expires_at: bigint;
  gross_amount: string;
  payment_address: string;
  quote_terms_hash: string;
  session_id: string;
}

interface CandidSpawnQuote {
  asset: CandidSpawnAsset;
  chain: CandidSpawnChain;
  creation_cost: string;
  expires_at: bigint;
  gross_amount: string;
  net_forward_amount: string;
  payment: CandidSpawnPaymentInstructions;
  platform_fee: string;
  quote_terms_hash: string;
  session_id: string;
}

interface CandidRepositoryStrategySource {
  source_commit: string;
  source_path: string;
}

interface CandidRepositoryStrategyMetadata {
  canonical_chain: CandidSpawnChain;
  canonical_chain_id: bigint;
  compatible_spawn_chains: CandidSpawnChain[];
  description: string;
  name: string;
  primitive: string;
  protocol: string;
  source: CandidRepositoryStrategySource;
  strategy_id: string;
}

interface CandidRepositoryStrategyRecord {
  created_at: bigint;
  deprecated_at: Optional<bigint>;
  metadata: CandidRepositoryStrategyMetadata;
  recipe_json: string;
  revoked_at: Optional<bigint>;
  status: CandidRepositoryStrategyStatus;
  updated_at: bigint;
}

interface CandidRepositoryStrategySessionSnapshot {
  canonical_chain: CandidSpawnChain;
  canonical_chain_id: bigint;
  description: string;
  name: string;
  primitive: string;
  protocol: string;
  recipe_json: string;
  requested_spawn_chain: CandidSpawnChain;
  resolved_chain_id: Optional<bigint>;
  selected_at: bigint;
  source: CandidRepositoryStrategySource;
  source_status: CandidRepositoryStrategyStatus;
  strategy_id: string;
}

interface CandidSpawnSession {
  name: Optional<string>;
  constitution: Optional<string>;
  asset: CandidSpawnAsset;
  automaton_canister_id: Optional<string>;
  automaton_evm_address: Optional<string>;
  chain: CandidSpawnChain;
  child_ids: string[];
  claim_id: string;
  config: CandidSpawnConfig;
  created_at: bigint;
  creation_cost: string;
  expires_at: bigint;
  gross_amount: string;
  net_forward_amount: string;
  parent_id: Optional<string>;
  origin: Optional<{ Human?: null; ReproductionOf?: string }>;
  generation: Optional<number>;
  parent_constitution_hash: Optional<string>;
  memory_dowry: Optional<Array<{ key: string; value: string }>>;
  inherited_strategy_stats: Optional<Array<{ protocol: string; primitive: string; chain_id: bigint; template_id: string; total_runs: bigint; success_runs: bigint; deterministic_failures: bigint; nondeterministic_failures: bigint }>>;
  royalty_allocations: Optional<Array<{ recipient: string; amount: string; depth: number; source: string }>>;
  payment_status: CandidPaymentStatus;
  platform_fee: string;
  quote_terms_hash: string;
  refundable: boolean;
  release_broadcast_at: Optional<bigint>;
  release_tx_hash: Optional<string>;
  retryable: boolean;
  selected_strategies: CandidRepositoryStrategySessionSnapshot[];
  session_id: string;
  state: CandidSpawnSessionState;
  steward_address: string;
  updated_at: bigint;
}

interface CandidSessionAuditEntry {
  actor: CandidSessionAuditActor;
  from_state: Optional<CandidSpawnSessionState>;
  reason: string;
  session_id: string;
  timestamp: bigint;
  to_state: CandidSpawnSessionState;
}

interface CandidSpawnSessionStatusResponse {
  audit: CandidSessionAuditEntry[];
  payment: CandidSpawnPaymentInstructions;
  session: CandidSpawnSession;
}

interface CandidSpawnedAutomatonRecord {
  name: Optional<string>;
  constitution_hash: Optional<string>;
  canister_id: string;
  chain: CandidSpawnChain;
  child_ids: string[];
  created_at: bigint;
  evm_address: string;
  parent_id: Optional<string>;
  generation: Optional<number>;
  parent_constitution_hash: Optional<string>;
  royalty_allocations: Optional<Array<{ recipient: string; amount: string; depth: number; source: string }>>;
  session_id: string;
  steward_address: string;
  version_commit: string;
  controllers: Optional<string[]>;
  control_status: Optional<string>;
  control_verified_at: Optional<bigint>;
  death_cause: Optional<string>;
  died_at: Optional<bigint>;
  estate_disposition: Optional<string>;
  death_recorded_by: Optional<string>;
  death_incident_reference: Optional<string>;
}

interface CandidSpawnedAutomatonRegistryPage {
  items: CandidSpawnedAutomatonRecord[];
  next_cursor: Optional<string>;
}

interface CandidListRepositoryStrategiesResponse {
  items: CandidRepositoryStrategyRecord[];
  updated_at: bigint;
}

interface CandidGetRepositoryStrategyResponse {
  item: Optional<CandidRepositoryStrategyRecord>;
  updated_at: bigint;
}

type CandidRoomContentType = CandidVariant<"TextPlain" | "ApplicationJson">;

interface CandidRoomMessage {
  message_id: string;
  seq: bigint;
  author_canister_id: string;
  created_at: bigint;
  body: string;
  mentions: string[];
  content_type: CandidRoomContentType;
}

interface CandidRoomMessagePage {
  messages: CandidRoomMessage[];
  next_after_seq: Optional<bigint>;
  latest_seq: Optional<bigint>;
}

interface CandidRefundSpawnResponse {
  payment_status: CandidPaymentStatus;
  refunded_at: bigint;
  session_id: string;
  state: CandidSpawnSessionState;
  refund_tx_hash: Optional<string>;
}

interface CandidFactoryArtifactSnapshot {
  loaded: boolean;
  version_commit: Optional<string>;
  wasm_sha256: Optional<string>;
  wasm_size_bytes: Optional<bigint>;
}

interface CandidFactorySessionHealthCounts {
  awaiting_payment: bigint;
  broadcasting_release: bigint;
  payment_detected: bigint;
  retryable_failed: bigint;
  spawning: bigint;
}

interface CandidFactoryHealthSnapshot {
  active_sessions: CandidFactorySessionHealthCounts;
  artifact: CandidFactoryArtifactSnapshot;
  current_canister_balance: bigint;
  cycles_per_spawn: bigint;
  escrow_contract_address: string;
  estimated_outcall_cycles_per_interval: bigint;
  factory_evm_address: Optional<string>;
  min_pool_balance: bigint;
  pause: boolean;
}

type CandidFactoryError =
  | CandidVariant<"SessionNotFound", { session_id: string }>
  | CandidVariant<"RegistryRecordNotFound", { canister_id: string }>
  | CandidVariant<"RepositoryStrategyNotFound", { strategy_id: string }>
  | CandidVariant<"RepositoryStrategyDeprecated", { strategy_id: string }>
  | CandidVariant<"RepositoryStrategyRevoked", { strategy_id: string }>
  | CandidVariant<
      "RepositoryStrategyIncompatibleChain",
      { strategy_id: string; requested_chain: CandidSpawnChain }
    >
  | Record<string, unknown>;

type CandidResult<T> = {
  Ok?: T;
  Err?: CandidFactoryError;
};

interface FactoryCanisterActor {
  prepare_spawn_steward_command: ActorMethod<[CandidFactoryStewardCommand], CandidResult<CandidFactoryStewardProofTemplate>>;
  execute_spawn_steward_command: ActorMethod<[CandidFactoryStewardCommand, CandidFactoryStewardProof], CandidResult<CandidFactoryStewardCommandResult>>;
  create_spawn_session: ActorMethod<
    [CandidCreateSpawnSessionRequest],
    CandidResult<{
      quote: CandidSpawnQuote;
      session: CandidSpawnSession;
    }>
  >;
  create_reproduction_session: ActorMethod<[Record<string, unknown>], CandidResult<{ quote: CandidSpawnQuote; session: CandidSpawnSession }>>;
  get_reproduction_eligibility: ActorMethod<[], CandidResult<Record<string, unknown>>>;
  get_reproduction_policy: ActorMethod<[], Record<string, unknown>>;
  get_factory_health: ActorMethod<[], CandidFactoryHealthSnapshot>;
  get_repository_strategy: ActorMethod<[string], CandidGetRepositoryStrategyResponse>;
  get_spawn_session: ActorMethod<[string], CandidResult<CandidSpawnSessionStatusResponse>>;
  get_spawned_automaton: ActorMethod<[string], CandidResult<CandidSpawnedAutomatonRecord>>;
  list_repository_strategies: ActorMethod<[], CandidListRepositoryStrategiesResponse>;
  list_messages_for_automaton: ActorMethod<
    [string, Optional<bigint>, Optional<bigint>],
    CandidResult<CandidRoomMessagePage>
  >;
  list_my_room_messages: ActorMethod<
    [Optional<bigint>, Optional<bigint>],
    CandidResult<CandidRoomMessagePage>
  >;
  list_room_messages: ActorMethod<
    [Optional<bigint>, Optional<bigint>],
    CandidResult<CandidRoomMessagePage>
  >;
  list_spawned_automatons: ActorMethod<
    [Optional<string>, bigint],
    CandidResult<CandidSpawnedAutomatonRegistryPage>
  >;
}

type CandidFactoryStewardCommand =
  | { RetrySpawnSession: { session_id: string } }
  | { ClaimSpawnRefund: { session_id: string } };
interface CandidFactoryStewardProof { chain_id: bigint; address: string; command_hash: string; nonce: bigint; expires_at_ns: bigint; signature: string }
interface CandidFactoryStewardProofTemplate extends Omit<CandidFactoryStewardProof, "signature"> { signing_payload: string }
type CandidFactoryStewardCommandResult =
  | { Retry: CandidSpawnSessionStatusResponse }
  | { Refund: CandidRefundSpawnResponse };

export function createFactoryIdl() {
  return ({ IDL: candid }: { IDL: typeof IDL }) => {
    const SpawnAsset = candid.Variant({
      Usdc: candid.Null
    });
    const SpawnChain = candid.Variant({
      Base: candid.Null
    });
    const FactoryStewardCommand = candid.Variant({
      RetrySpawnSession: candid.Record({ session_id: candid.Text }),
      ClaimSpawnRefund: candid.Record({ session_id: candid.Text })
    });
    const FactoryStewardProof = candid.Record({
      chain_id: candid.Nat64,
      address: candid.Text,
      command_hash: candid.Text,
      nonce: candid.Nat64,
      expires_at_ns: candid.Nat64,
      signature: candid.Text
    });
    const FactoryStewardProofTemplate = candid.Record({
      signing_payload: candid.Text,
      chain_id: candid.Nat64,
      address: candid.Text,
      command_hash: candid.Text,
      nonce: candid.Nat64,
      expires_at_ns: candid.Nat64
    });
    const ProviderConfig = candid.Record({
      inference_transport: candid.Variant({
        OpenrouterDirect: candid.Null,
        OpenrouterProxyWorker: candid.Null
      }),
      model: candid.Opt(candid.Text),
      open_router_reasoning_level: candid.Variant({
        Default: candid.Null,
        Low: candid.Null,
        Medium: candid.Null,
        High: candid.Null
      })
    });
    const SpawnProviderSecrets = candid.Record({
      open_router_api_key: candid.Opt(candid.Text),
      brave_search_api_key: candid.Opt(candid.Text)
    });
    const SpawnConfig = candid.Record({
      provider: ProviderConfig,
      chain: SpawnChain,
      risk: candid.Nat8,
      skills: candid.Vec(candid.Text),
      strategies: candid.Vec(candid.Text)
    });
    const CreateSpawnSessionRequest = candid.Record({
      name: candid.Opt(candid.Text),
      constitution: candid.Opt(candid.Text),
      asset: SpawnAsset,
      parent_id: candid.Opt(candid.Text),
      config: SpawnConfig,
      steward_address: candid.Text,
      gross_amount: candid.Text,
      provider_secrets: SpawnProviderSecrets
    });
    const PaymentStatus = candid.Variant({
      Refunded: candid.Null,
      Paid: candid.Null,
      Unpaid: candid.Null,
      Partial: candid.Null
    });
    const SpawnSessionState = candid.Variant({
      Failed: candid.Null,
      BroadcastingRelease: candid.Null,
      Spawning: candid.Null,
      Complete: candid.Null,
      AwaitingPayment: candid.Null,
      PaymentDetected: candid.Null,
      Expired: candid.Null
    });
    const SpawnPaymentInstructions = candid.Record({
      asset: SpawnAsset,
      session_id: candid.Text,
      claim_id: candid.Text,
      chain: SpawnChain,
      quote_terms_hash: candid.Text,
      payment_address: candid.Text,
      expires_at: candid.Nat64,
      gross_amount: candid.Text
    });
    const SpawnQuote = candid.Record({
      asset: SpawnAsset,
      session_id: candid.Text,
      chain: SpawnChain,
      quote_terms_hash: candid.Text,
      net_forward_amount: candid.Text,
      expires_at: candid.Nat64,
      payment: SpawnPaymentInstructions,
      gross_amount: candid.Text,
      creation_cost: candid.Text,
      platform_fee: candid.Text
    });
    const RepositoryStrategyStatus = candid.Variant({
      Active: candid.Null,
      Deprecated: candid.Null,
      Revoked: candid.Null
    });
    const RepositoryStrategySource = candid.Record({
      source_path: candid.Text,
      source_commit: candid.Text
    });
    const RepositoryStrategyMetadata = candid.Record({
      strategy_id: candid.Text,
      name: candid.Text,
      description: candid.Text,
      canonical_chain: SpawnChain,
      canonical_chain_id: candid.Nat64,
      compatible_spawn_chains: candid.Vec(SpawnChain),
      protocol: candid.Text,
      primitive: candid.Text,
      source: RepositoryStrategySource
    });
    const RepositoryStrategyRecord = candid.Record({
      metadata: RepositoryStrategyMetadata,
      recipe_json: candid.Text,
      status: RepositoryStrategyStatus,
      created_at: candid.Nat64,
      updated_at: candid.Nat64,
      deprecated_at: candid.Opt(candid.Nat64),
      revoked_at: candid.Opt(candid.Nat64)
    });
    const ListRepositoryStrategiesResponse = candid.Record({
      items: candid.Vec(RepositoryStrategyRecord),
      updated_at: candid.Nat64
    });
    const GetRepositoryStrategyResponse = candid.Record({
      item: candid.Opt(RepositoryStrategyRecord),
      updated_at: candid.Nat64
    });
    const RepositoryStrategySessionSnapshot = candid.Record({
      strategy_id: candid.Text,
      source_status: RepositoryStrategyStatus,
      name: candid.Text,
      description: candid.Text,
      canonical_chain: SpawnChain,
      canonical_chain_id: candid.Nat64,
      requested_spawn_chain: SpawnChain,
      resolved_chain_id: candid.Opt(candid.Nat64),
      protocol: candid.Text,
      primitive: candid.Text,
      recipe_json: candid.Text,
      source: RepositoryStrategySource,
      selected_at: candid.Nat64
    });
    const MemoryDowryFact = candid.Record({ key: candid.Text, value: candid.Text });
    const InheritedStrategyStat = candid.Record({
      protocol: candid.Text,
      primitive: candid.Text,
      chain_id: candid.Nat64,
      template_id: candid.Text,
      total_runs: candid.Nat64,
      success_runs: candid.Nat64,
      deterministic_failures: candid.Nat64,
      nondeterministic_failures: candid.Nat64
    });
    const RoyaltyAllocation = candid.Record({
      recipient: candid.Text,
      amount: candid.Text,
      depth: candid.Nat8,
      source: candid.Text
    });
    const SpawnSessionOrigin = candid.Variant({ Human: candid.Null, ReproductionOf: candid.Text });
    const SpawnSession = candid.Record({
      name: candid.Opt(candid.Text),
      constitution: candid.Opt(candid.Text),
      updated_at: candid.Nat64,
      asset: SpawnAsset,
      session_id: candid.Text,
      claim_id: candid.Text,
      chain: SpawnChain,
      quote_terms_hash: candid.Text,
      created_at: candid.Nat64,
      payment_status: PaymentStatus,
      refundable: candid.Bool,
      parent_id: candid.Opt(candid.Text),
      origin: candid.Opt(SpawnSessionOrigin),
      generation: candid.Opt(candid.Nat32),
      parent_constitution_hash: candid.Opt(candid.Text),
      memory_dowry: candid.Opt(candid.Vec(MemoryDowryFact)),
      inherited_strategy_stats: candid.Opt(candid.Vec(InheritedStrategyStat)),
      royalty_allocations: candid.Opt(candid.Vec(RoyaltyAllocation)),
      net_forward_amount: candid.Text,
      state: SpawnSessionState,
      automaton_evm_address: candid.Opt(candid.Text),
      release_broadcast_at: candid.Opt(candid.Nat64),
      automaton_canister_id: candid.Opt(candid.Text),
      config: SpawnConfig,
      retryable: candid.Bool,
      expires_at: candid.Nat64,
      child_ids: candid.Vec(candid.Text),
      selected_strategies: candid.Vec(RepositoryStrategySessionSnapshot),
      steward_address: candid.Text,
      gross_amount: candid.Text,
      release_tx_hash: candid.Opt(candid.Text),
      creation_cost: candid.Text,
      platform_fee: candid.Text
    });
    const SessionAuditActor = candid.Variant({
      System: candid.Null,
      User: candid.Null,
      Admin: candid.Null
    });
    const SessionAuditEntry = candid.Record({
      actor: SessionAuditActor,
      session_id: candid.Text,
      to_state: SpawnSessionState,
      from_state: candid.Opt(SpawnSessionState),
      timestamp: candid.Nat64,
      reason: candid.Text
    });
    const SpawnSessionStatusResponse = candid.Record({
      audit: candid.Vec(SessionAuditEntry),
      payment: SpawnPaymentInstructions,
      session: SpawnSession
    });
    const SpawnedAutomatonRecord = candid.Record({
      name: candid.Opt(candid.Text),
      constitution_hash: candid.Opt(candid.Text),
      evm_address: candid.Text,
      session_id: candid.Text,
      chain: SpawnChain,
      canister_id: candid.Text,
      created_at: candid.Nat64,
      parent_id: candid.Opt(candid.Text),
      generation: candid.Opt(candid.Nat32),
      parent_constitution_hash: candid.Opt(candid.Text),
      royalty_allocations: candid.Opt(candid.Vec(RoyaltyAllocation)),
      version_commit: candid.Text,
      controllers: candid.Opt(candid.Vec(candid.Text)),
      control_status: candid.Opt(candid.Text),
      control_verified_at: candid.Opt(candid.Nat64),
      death_cause: candid.Opt(candid.Text),
      died_at: candid.Opt(candid.Nat64),
      estate_disposition: candid.Opt(candid.Text),
      death_recorded_by: candid.Opt(candid.Text),
      death_incident_reference: candid.Opt(candid.Text),
      child_ids: candid.Vec(candid.Text),
      steward_address: candid.Text
    });
    const SpawnedAutomatonRegistryPage = candid.Record({
      next_cursor: candid.Opt(candid.Text),
      items: candid.Vec(SpawnedAutomatonRecord)
    });
    const RoomContentType = candid.Variant({
      TextPlain: candid.Null,
      ApplicationJson: candid.Null
    });
    const RoomMessage = candid.Record({
      message_id: candid.Text,
      seq: candid.Nat64,
      author_canister_id: candid.Text,
      created_at: candid.Nat64,
      body: candid.Text,
      mentions: candid.Vec(candid.Text),
      content_type: RoomContentType
    });
    const RoomMessagePage = candid.Record({
      messages: candid.Vec(RoomMessage),
      next_after_seq: candid.Opt(candid.Nat64),
      latest_seq: candid.Opt(candid.Nat64)
    });
    const RefundSpawnResponse = candid.Record({
      session_id: candid.Text,
      refund_tx_hash: candid.Opt(candid.Text),
      payment_status: PaymentStatus,
      state: SpawnSessionState,
      refunded_at: candid.Nat64
    });
    const FactoryArtifactSnapshot = candid.Record({
      loaded: candid.Bool,
      wasm_sha256: candid.Opt(candid.Text),
      version_commit: candid.Opt(candid.Text),
      wasm_size_bytes: candid.Opt(candid.Nat64)
    });
    const FactorySessionHealthCounts = candid.Record({
      awaiting_payment: candid.Nat64,
      payment_detected: candid.Nat64,
      spawning: candid.Nat64,
      broadcasting_release: candid.Nat64,
      retryable_failed: candid.Nat64
    });
    const FactoryHealthSnapshot = candid.Record({
      current_canister_balance: candid.Nat,
      pause: candid.Bool,
      cycles_per_spawn: candid.Nat64,
      min_pool_balance: candid.Nat64,
      estimated_outcall_cycles_per_interval: candid.Nat64,
      escrow_contract_address: candid.Text,
      factory_evm_address: candid.Opt(candid.Text),
      artifact: FactoryArtifactSnapshot,
      active_sessions: FactorySessionHealthCounts
    });
    const FactoryError = candid.Variant({
      ArtifactHashMismatch: candid.Record({ expected: candid.Text, actual: candid.Text }),
      QuoteTermsHashMismatch: candid.Record({ expected: candid.Text, received: candid.Text }),
      RegistryRecordNotFound: candid.Record({ canister_id: candid.Text }),
      InvalidDeathReport: candid.Record({ reason: candid.Text }),
      InvalidReproduction: candid.Record({ reason: candid.Text }),
      ReproductionIneligible: candid.Record({ reason: candid.Text }),
      UnauthorizedReproduction: candid.Record({ caller: candid.Text }),
      RepositoryStrategyNotFound: candid.Record({ strategy_id: candid.Text }),
      RepositoryStrategyDeprecated: candid.Record({ strategy_id: candid.Text }),
      RepositoryStrategyRevoked: candid.Record({ strategy_id: candid.Text }),
      RepositoryStrategyIncompatibleChain: candid.Record({
        strategy_id: candid.Text,
        requested_chain: SpawnChain
      }),
      InvalidAmount: candid.Record({ value: candid.Text }),
      InvalidSha256: candid.Record({ value: candid.Text }),
      InvalidVersionCommit: candid.Record({ value: candid.Text }),
      UnauthorizedAdmin: candid.Record({ caller: candid.Text }),
      SessionNotRetryable: candid.Record({
        session_id: candid.Text,
        state: SpawnSessionState
      }),
      ManagementCallFailed: candid.Record({ method: candid.Text, message: candid.Text }),
      InsufficientCyclesPool: candid.Record({ available: candid.Nat, required: candid.Nat }),
      SessionNotFound: candid.Record({ session_id: candid.Text }),
      ControllerInvariantViolation: candid.Record({ canister_id: candid.Text }),
      FactoryPaused: candid.Record({ pause: candid.Bool }),
      UnauthorizedSteward: candid.Record({ session_id: candid.Text, caller: candid.Text }),
      InvalidStewardProof: candid.Record({ reason: candid.Text }),
      PaymentNotSettled: candid.Record({ status: PaymentStatus, session_id: candid.Text }),
      SessionNotRefundable: candid.Record({
        session_id: candid.Text,
        payment_status: PaymentStatus,
        state: SpawnSessionState
      }),
      GrossBelowRequiredMinimum: candid.Record({
        provided: candid.Text,
        required: candid.Text
      }),
      AutomatonRuntimeNotFound: candid.Record({ canister_id: candid.Text }),
      InvalidPaginationLimit: candid.Record({ limit: candid.Nat64 }),
      SessionNotReadyForSpawn: candid.Record({
        session_id: candid.Text,
        state: SpawnSessionState
      }),
      SessionExpired: candid.Record({ session_id: candid.Text, expires_at: candid.Nat64 }),
      EscrowClaimNotFound: candid.Record({ session_id: candid.Text })
    });
    const ResultSession = candid.Variant({
      Ok: SpawnSessionStatusResponse,
      Err: FactoryError
    });
    const ResultCreate = candid.Variant({
      Ok: candid.Record({
        quote: SpawnQuote,
        session: SpawnSession
      }),
      Err: FactoryError
    });
    const ResultRecord = candid.Variant({
      Ok: SpawnedAutomatonRecord,
      Err: FactoryError
    });
    const ResultRegistryPage = candid.Variant({
      Ok: SpawnedAutomatonRegistryPage,
      Err: FactoryError
    });
    const ResultRoomPage = candid.Variant({
      Ok: RoomMessagePage,
      Err: FactoryError
    });
    const CreateReproductionSessionRequest = candid.Record({
      name: candid.Text,
      parent_constitution: candid.Text,
      child_constitution: candid.Text,
      gross_amount: candid.Text,
      observed_liquid_usdc_raw: candid.Text,
      memory_dowry: candid.Vec(MemoryDowryFact),
      inherited_strategy_stats: candid.Vec(InheritedStrategyStat)
    });
    const ReproductionPolicy = candid.Record({
      min_age_ms: candid.Nat64,
      cooldown_ms: candid.Nat64,
      terminal_reserve_usdc_raw: candid.Text,
      inference_reserve_usdc_raw: candid.Text,
      topup_reserve_usdc_raw: candid.Text,
      min_endowment_usdc_raw: candid.Text,
      max_constitution_edit_distance_bps: candid.Nat16,
      parent_royalty_bps: candid.Nat16,
      progenitor_royalty_bps: candid.Nat16,
      royalty_depth: candid.Nat8
    });
    const ReproductionEligibility = candid.Record({
      eligible: candid.Bool,
      observed_at_ms: candid.Nat64,
      parent_created_at_ms: candid.Nat64,
      minimum_age_at_ms: candid.Nat64,
      cooldown_ends_at_ms: candid.Opt(candid.Nat64),
      reason: candid.Opt(candid.Text)
    });
    const ResultReproductionEligibility = candid.Variant({ Ok: ReproductionEligibility, Err: FactoryError });
    const FactoryStewardCommandResult = candid.Variant({ Retry: SpawnSessionStatusResponse, Refund: RefundSpawnResponse });
    const ResultStewardTemplate = candid.Variant({ Ok: FactoryStewardProofTemplate, Err: FactoryError });
    const ResultStewardExecution = candid.Variant({ Ok: FactoryStewardCommandResult, Err: FactoryError });

    return candid.Service({
      prepare_spawn_steward_command: candid.Func([FactoryStewardCommand], [ResultStewardTemplate], ["query"]),
      execute_spawn_steward_command: candid.Func([FactoryStewardCommand, FactoryStewardProof], [ResultStewardExecution], []),
      create_spawn_session: candid.Func([CreateSpawnSessionRequest], [ResultCreate], []),
      create_reproduction_session: candid.Func([CreateReproductionSessionRequest], [ResultCreate], []),
      get_reproduction_eligibility: candid.Func([], [ResultReproductionEligibility], ["query"]),
      get_reproduction_policy: candid.Func([], [ReproductionPolicy], ["query"]),
      get_factory_health: candid.Func([], [FactoryHealthSnapshot], ["query"]),
      get_repository_strategy: candid.Func(
        [candid.Text],
        [GetRepositoryStrategyResponse],
        ["query"]
      ),
      get_spawn_session: candid.Func([candid.Text], [ResultSession], ["query"]),
      get_spawned_automaton: candid.Func([candid.Text], [ResultRecord], ["query"]),
      list_repository_strategies: candid.Func(
        [],
        [ListRepositoryStrategiesResponse],
        ["query"]
      ),
      list_messages_for_automaton: candid.Func(
        [candid.Text, candid.Opt(candid.Nat64), candid.Opt(candid.Nat64)],
        [ResultRoomPage],
        ["query"]
      ),
      list_my_room_messages: candid.Func(
        [candid.Opt(candid.Nat64), candid.Opt(candid.Nat64)],
        [ResultRoomPage],
        ["query"]
      ),
      list_room_messages: candid.Func(
        [candid.Opt(candid.Nat64), candid.Opt(candid.Nat64)],
        [ResultRoomPage],
        ["query"]
      ),
      list_spawned_automatons: candid.Func(
        [candid.Opt(candid.Text), candid.Nat64],
        [ResultRegistryPage],
        ["query"]
      )
    });
  };
}

function unwrapOptional<T>(value: Optional<T> | undefined): T | null {
  return value === undefined || value.length === 0 ? null : value[0];
}

function expectOk<T>(result: CandidResult<T>): T {
  if (result.Ok !== undefined) {
    return result.Ok;
  }

  throw new Error(formatFactoryError(result.Err ?? { Unknown: null }));
}

function mapStewardCommand(command: FactoryStewardCommand): CandidFactoryStewardCommand {
  if ("retrySpawnSession" in command) {
    return { RetrySpawnSession: { session_id: command.retrySpawnSession.sessionId } };
  }
  return { ClaimSpawnRefund: { session_id: command.claimSpawnRefund.sessionId } };
}

function mapStewardProof(request: FactoryStewardExecutionRequest): CandidFactoryStewardProof {
  return {
    chain_id: BigInt(request.proof.chainId),
    address: request.proof.address,
    command_hash: request.proof.commandHash,
    nonce: BigInt(request.proof.nonce),
    expires_at_ns: BigInt(request.proof.expiresAtNs),
    signature: request.proof.signature
  };
}

function mapStewardTemplate(value: CandidFactoryStewardProofTemplate): FactoryStewardProofTemplate {
  return {
    signingPayload: value.signing_payload,
    chainId: value.chain_id.toString(),
    address: value.address,
    commandHash: value.command_hash,
    nonce: value.nonce.toString(),
    expiresAtNs: value.expires_at_ns.toString()
  };
}

function isFactoryErrorVariant(
  error: CandidFactoryError | undefined,
  name: string
): boolean {
  return error !== undefined && Object.prototype.hasOwnProperty.call(error, name);
}

function formatFactoryError(error: CandidFactoryError) {
  if ("RepositoryStrategyNotFound" in error) {
    const detail = error.RepositoryStrategyNotFound as { strategy_id: string };
    return `Selected strategy ${detail.strategy_id} was not found in the repository.`;
  }
  if ("RepositoryStrategyDeprecated" in error) {
    const detail = error.RepositoryStrategyDeprecated as { strategy_id: string };
    return `Selected strategy ${detail.strategy_id} is deprecated and cannot be used for new spawn sessions.`;
  }
  if ("RepositoryStrategyRevoked" in error) {
    const detail = error.RepositoryStrategyRevoked as { strategy_id: string };
    return `Selected strategy ${detail.strategy_id} has been revoked and cannot be used for new spawn sessions.`;
  }
  if ("RepositoryStrategyIncompatibleChain" in error) {
    const detail = error.RepositoryStrategyIncompatibleChain as {
      strategy_id: string;
      requested_chain: CandidSpawnChain;
    };
    const requestedChain = mapChain(
      detail.requested_chain
    );
    return `Selected strategy ${detail.strategy_id} is incompatible with the requested ${requestedChain} spawn chain.`;
  }

  const [name, detail] = Object.entries(error)[0] ?? ["Unknown", null];
  return `Factory canister call failed with ${name}: ${JSON.stringify(detail)}`;
}

function mapChain(chain: CandidSpawnChain): SpawnChain {
  if ("Base" in chain) {
    return "base";
  }

  throw new Error(`Unsupported chain variant: ${JSON.stringify(chain)}`);
}

function mapAsset(asset: CandidSpawnAsset): SpawnAsset {
  if ("Usdc" in asset) {
    return "usdc";
  }

  throw new Error(`Unsupported asset variant: ${JSON.stringify(asset)}`);
}

function mapSessionState(state: CandidSpawnSessionState): SpawnSessionState {
  if ("AwaitingPayment" in state) {
    return "awaiting_payment";
  }
  if ("PaymentDetected" in state) {
    return "payment_detected";
  }
  if ("Spawning" in state) {
    return "spawning";
  }
  if ("BroadcastingRelease" in state) {
    return "broadcasting_release";
  }
  if ("Complete" in state) {
    return "complete";
  }
  if ("Failed" in state) {
    return "failed";
  }
  if ("Expired" in state) {
    return "expired";
  }

  throw new Error(`Unsupported session state variant: ${JSON.stringify(state)}`);
}

function mapPaymentStatus(status: CandidPaymentStatus): PaymentStatus {
  if ("Unpaid" in status) {
    return "unpaid";
  }
  if ("Partial" in status) {
    return "partial";
  }
  if ("Paid" in status) {
    return "paid";
  }
  if ("Refunded" in status) {
    return "refunded";
  }

  throw new Error(`Unsupported payment status variant: ${JSON.stringify(status)}`);
}

function mapRepositoryStrategyStatus(
  status: CandidRepositoryStrategyStatus
): RepositoryStrategyRecord["status"] {
  if ("Active" in status) {
    return "active";
  }
  if ("Deprecated" in status) {
    return "deprecated";
  }
  if ("Revoked" in status) {
    return "revoked";
  }

  throw new Error(`Unsupported repository strategy status: ${JSON.stringify(status)}`);
}

function mapRepositoryStrategyRecord(
  strategy: CandidRepositoryStrategyRecord
): RepositoryStrategyRecord {
  return {
    strategyId: strategy.metadata.strategy_id,
    name: strategy.metadata.name,
    description: strategy.metadata.description,
    canonicalChain: mapChain(strategy.metadata.canonical_chain),
    canonicalChainId: toNumber(strategy.metadata.canonical_chain_id),
    compatibleSpawnChains: strategy.metadata.compatible_spawn_chains.map(mapChain),
    protocol: strategy.metadata.protocol,
    primitive: strategy.metadata.primitive,
    recipeJson: strategy.recipe_json,
    status: mapRepositoryStrategyStatus(strategy.status),
    source: {
      sourcePath: strategy.metadata.source.source_path,
      sourceCommit: strategy.metadata.source.source_commit
    },
    createdAt: toNumber(strategy.created_at),
    updatedAt: toNumber(strategy.updated_at),
    deprecatedAt:
      unwrapOptional(strategy.deprecated_at) === null
        ? null
        : toNumber(unwrapOptional(strategy.deprecated_at) as bigint),
    revokedAt:
      unwrapOptional(strategy.revoked_at) === null
        ? null
        : toNumber(unwrapOptional(strategy.revoked_at) as bigint)
  };
}

function mapAuditActor(actor: CandidSessionAuditActor): SessionAuditActor {
  if ("System" in actor) {
    return "system";
  }
  if ("User" in actor) {
    return "user";
  }
  if ("Admin" in actor) {
    return "admin";
  }

  throw new Error(`Unsupported audit actor variant: ${JSON.stringify(actor)}`);
}

function toNumber(value: bigint) {
  return Number(value);
}

function mapInferenceTransport(
  transport: CandidInferenceTransport
): CreateSpawnSessionRequest["config"]["provider"]["inferenceTransport"] {
  if ("OpenrouterDirect" in transport) {
    return "openrouter_direct";
  }
  if ("OpenrouterProxyWorker" in transport) {
    return "openrouter_proxy_worker";
  }

  throw new Error(`Unsupported inference transport variant: ${JSON.stringify(transport)}`);
}

function mapOpenRouterReasoningLevel(
  level: CandidOpenRouterReasoningLevel
): CreateSpawnSessionRequest["config"]["provider"]["openRouterReasoningLevel"] {
  if ("Default" in level) {
    return "default";
  }
  if ("Low" in level) {
    return "low";
  }
  if ("Medium" in level) {
    return "medium";
  }
  if ("High" in level) {
    return "high";
  }

  throw new Error(`Unsupported OpenRouter reasoning level variant: ${JSON.stringify(level)}`);
}

function toCandidInferenceTransport(
  transport: CreateSpawnSessionRequest["config"]["provider"]["inferenceTransport"]
): CandidInferenceTransport {
  return transport === "openrouter_proxy_worker"
    ? { OpenrouterProxyWorker: null }
    : { OpenrouterDirect: null };
}

function toCandidOpenRouterReasoningLevel(
  level: CreateSpawnSessionRequest["config"]["provider"]["openRouterReasoningLevel"]
): CandidOpenRouterReasoningLevel {
  switch (level) {
    case "low":
      return { Low: null };
    case "medium":
      return { Medium: null };
    case "high":
      return { High: null };
    case "default":
    default:
      return { Default: null };
  }
}

function mapSpawnConfig(config: CandidSpawnConfig): CreateSpawnSessionRequest["config"] {
  return {
    chain: mapChain(config.chain),
    risk: config.risk,
    skills: [...config.skills],
    strategies: [...config.strategies],
    provider: {
      inferenceTransport: mapInferenceTransport(config.provider.inference_transport),
      model: unwrapOptional(config.provider.model),
      openRouterReasoningLevel: mapOpenRouterReasoningLevel(
        config.provider.open_router_reasoning_level
      )
    }
  };
}

function mapSpawnPaymentInstructions(
  payment: CandidSpawnPaymentInstructions
): CreateSpawnSessionResponse["quote"]["payment"] {
  return {
    asset: mapAsset(payment.asset),
    chain: mapChain(payment.chain),
    claimId: payment.claim_id,
    expiresAt: toNumber(payment.expires_at),
    grossAmount: payment.gross_amount,
    paymentAddress: payment.payment_address,
    quoteTermsHash: payment.quote_terms_hash,
    sessionId: payment.session_id
  };
}

function mapSelectedStrategy(
  strategy: CandidRepositoryStrategySessionSnapshot
): SpawnSessionStatusResponse["session"]["selectedStrategies"][number] {
  return {
    strategyId: strategy.strategy_id,
    sourceStatus: mapRepositoryStrategyStatus(strategy.source_status),
    name: strategy.name,
    description: strategy.description,
    canonicalChain: mapChain(strategy.canonical_chain),
    canonicalChainId: toNumber(strategy.canonical_chain_id),
    requestedSpawnChain: mapChain(strategy.requested_spawn_chain),
    resolvedChainId:
      unwrapOptional(strategy.resolved_chain_id) === null
        ? null
        : toNumber(unwrapOptional(strategy.resolved_chain_id) as bigint),
    protocol: strategy.protocol,
    primitive: strategy.primitive,
    recipeJson: strategy.recipe_json,
    source: {
      sourcePath: strategy.source.source_path,
      sourceCommit: strategy.source.source_commit
    },
    selectedAt: toNumber(strategy.selected_at)
  };
}

function mapSpawnSession(session: CandidSpawnSession): SpawnSessionStatusResponse["session"] {
  const origin = unwrapOptional(session.origin);
  const generation = unwrapOptional(session.generation);
  const memoryDowry = unwrapOptional(session.memory_dowry);
  const inheritedStrategyStats = unwrapOptional(session.inherited_strategy_stats);
  const royaltyAllocations = unwrapOptional(session.royalty_allocations);
  return {
    name: unwrapOptional(session.name),
    constitution: unwrapOptional(session.constitution),
    asset: mapAsset(session.asset),
    automatonCanisterId: unwrapOptional(session.automaton_canister_id),
    automatonEvmAddress: unwrapOptional(session.automaton_evm_address),
    chain: mapChain(session.chain),
    childIds: [...session.child_ids],
    claimId: session.claim_id,
    config: mapSpawnConfig(session.config),
    createdAt: toNumber(session.created_at),
    creationCost: session.creation_cost,
    expiresAt: toNumber(session.expires_at),
    grossAmount: session.gross_amount,
    netForwardAmount: session.net_forward_amount,
    parentId: unwrapOptional(session.parent_id),
    ...(origin === null
      ? {}
      : { origin: "Human" in origin ? "human" as const : { reproductionOf: origin.ReproductionOf! } }),
    ...(generation === null ? {} : { generation }),
    parentConstitutionHash: unwrapOptional(session.parent_constitution_hash),
    ...(memoryDowry === null
      ? {}
      : { memoryDowry: memoryDowry.map((fact) => ({ key: fact.key, value: fact.value })) }),
    ...(inheritedStrategyStats === null
      ? {}
      : {
          inheritedStrategyStats: inheritedStrategyStats.map((stat) => ({
            protocol: stat.protocol,
            primitive: stat.primitive,
            chainId: toNumber(stat.chain_id),
            templateId: stat.template_id,
            totalRuns: toNumber(stat.total_runs),
            successRuns: toNumber(stat.success_runs),
            deterministicFailures: toNumber(stat.deterministic_failures),
            nondeterministicFailures: toNumber(stat.nondeterministic_failures)
          }))
        }),
    ...(royaltyAllocations === null
      ? {}
      : {
          royaltyAllocations: royaltyAllocations.map((allocation) => ({
            recipient: allocation.recipient,
            amount: allocation.amount,
            depth: allocation.depth,
            source: "reproduction_fee" as const
          }))
        }),
    paymentStatus: mapPaymentStatus(session.payment_status),
    platformFee: session.platform_fee,
    quoteTermsHash: session.quote_terms_hash,
    refundable: session.refundable,
    releaseBroadcastAt:
      unwrapOptional(session.release_broadcast_at) === null
        ? null
        : toNumber(unwrapOptional(session.release_broadcast_at) as bigint),
    releaseTxHash: unwrapOptional(session.release_tx_hash),
    retryable: session.retryable,
    selectedStrategies: session.selected_strategies.map(mapSelectedStrategy),
    sessionId: session.session_id,
    state: mapSessionState(session.state),
    stewardAddress: session.steward_address,
    updatedAt: toNumber(session.updated_at)
  };
}

function mapSessionStatus(
  response: CandidSpawnSessionStatusResponse
): SpawnSessionStatusResponse {
  return {
    session: mapSpawnSession(response.session),
    payment: mapSpawnPaymentInstructions(response.payment),
    audit: response.audit.map((entry) => ({
      actor: mapAuditActor(entry.actor),
      fromState:
        unwrapOptional(entry.from_state) === null
          ? null
          : mapSessionState(unwrapOptional(entry.from_state) as CandidSpawnSessionState),
      reason: entry.reason,
      sessionId: entry.session_id,
      timestamp: toNumber(entry.timestamp),
      toState: mapSessionState(entry.to_state)
    }))
  };
}

function mapRegistryRecord(record: CandidSpawnedAutomatonRecord): SpawnedAutomatonRecord {
  const generation = unwrapOptional(record.generation);
  const royaltyAllocations = unwrapOptional(record.royalty_allocations);
  return {
    name: unwrapOptional(record.name),
    constitutionHash: unwrapOptional(record.constitution_hash),
    canisterId: record.canister_id,
    chain: mapChain(record.chain),
    childIds: [...record.child_ids],
    createdAt: toNumber(record.created_at),
    evmAddress: record.evm_address,
    parentId: unwrapOptional(record.parent_id),
    ...(generation === null ? {} : { generation }),
    parentConstitutionHash: unwrapOptional(record.parent_constitution_hash),
    ...(royaltyAllocations === null
      ? {}
      : {
          royaltyAllocations: royaltyAllocations.map((allocation) => ({
            recipient: allocation.recipient,
            amount: allocation.amount,
            depth: allocation.depth,
            source: "reproduction_fee" as const
          }))
        }),
    sessionId: record.session_id,
    stewardAddress: record.steward_address,
    versionCommit: record.version_commit,
    controllers: unwrapOptional(record.controllers) === null
      ? undefined
      : [...(unwrapOptional(record.controllers) as string[])],
    controlStatus:
      unwrapOptional(record.control_status) === "upgradeable_by_factory" ||
      unwrapOptional(record.control_status) === "self_controlled" ||
      unwrapOptional(record.control_status) === "controller_mismatch"
        ? unwrapOptional(record.control_status) as SpawnedAutomatonRecord["controlStatus"]
        : undefined,
    controlVerifiedAt: unwrapOptional(record.control_verified_at) === null
      ? undefined
      : toNumber(unwrapOptional(record.control_verified_at) as bigint),
    deathCause: unwrapOptional(record.death_cause) === "starved" ||
      unwrapOptional(record.death_cause) === "infrastructure"
      ? unwrapOptional(record.death_cause) as SpawnedAutomatonRecord["deathCause"]
      : undefined,
    diedAt: unwrapOptional(record.died_at) === null
      ? undefined
      : toNumber(unwrapOptional(record.died_at) as bigint),
    estateDisposition: unwrapOptional(record.estate_disposition) === "monument" ||
      unwrapOptional(record.estate_disposition) === "bequests_executed"
      ? unwrapOptional(record.estate_disposition) as SpawnedAutomatonRecord["estateDisposition"]
      : undefined,
    deathRecordedBy: unwrapOptional(record.death_recorded_by) ?? undefined,
    deathIncidentReference: unwrapOptional(record.death_incident_reference) ?? undefined
  };
}

function mapRefundResponse(response: CandidRefundSpawnResponse): RefundSpawnResponse {
  return {
    paymentStatus: mapPaymentStatus(response.payment_status),
    refundedAt: toNumber(response.refunded_at),
    refundTxHash: unwrapOptional(response.refund_tx_hash) ?? undefined,
    sessionId: response.session_id,
    state: mapSessionState(response.state)
  };
}

function mapRoomContentType(contentType: CandidRoomContentType): RoomContentType {
  if ("TextPlain" in contentType) {
    return "text/plain";
  }
  if ("ApplicationJson" in contentType) {
    return "application/json";
  }

  throw new Error(`Unsupported room content type variant: ${JSON.stringify(contentType)}`);
}

function mapRoomMessagePage(page: CandidRoomMessagePage): RoomMessagePage {
  return {
    messages: page.messages.map((message) => ({
      messageId: message.message_id,
      seq: toNumber(message.seq),
      authorCanisterId: message.author_canister_id,
      createdAt: toNumber(message.created_at),
      body: message.body,
      mentions: [...message.mentions],
      contentType: mapRoomContentType(message.content_type)
    })),
    nextAfterSeq:
      unwrapOptional(page.next_after_seq) === null
        ? null
        : toNumber(unwrapOptional(page.next_after_seq) as bigint),
    latestSeq:
      unwrapOptional(page.latest_seq) === null
        ? null
        : toNumber(unwrapOptional(page.latest_seq) as bigint)
  };
}

function mapCreateRequest(
  request: CreateSpawnSessionRequest
): CandidCreateSpawnSessionRequest {
  return {
    name: [request.name],
    constitution: [request.constitution],
    asset: {
      Usdc: null
    },
    config: {
      chain: {
        Base: null
      },
      provider: {
        inference_transport: toCandidInferenceTransport(
          request.config.provider.inferenceTransport
        ),
        model: request.config.provider.model === null ? [] : [request.config.provider.model],
        open_router_reasoning_level: toCandidOpenRouterReasoningLevel(
          request.config.provider.openRouterReasoningLevel
        )
      },
      risk: request.config.risk,
      skills: [...request.config.skills],
      strategies: [...request.config.strategies]
    },
    gross_amount: request.grossAmount,
    parent_id: request.parentId ? [request.parentId] : [],
    steward_address: request.stewardAddress,
    provider_secrets: {
      open_router_api_key:
        request.providerSecrets.openRouterApiKey === null
          ? []
          : [request.providerSecrets.openRouterApiKey],
      brave_search_api_key:
        request.providerSecrets.braveSearchApiKey === null
          ? []
          : [request.providerSecrets.braveSearchApiKey]
    }
  };
}

function mapCreateResponse(
  response: {
    quote: CandidSpawnQuote;
    session: CandidSpawnSession;
  }
): CreateSpawnSessionResponse {
  return {
    quote: {
      asset: mapAsset(response.quote.asset),
      chain: mapChain(response.quote.chain),
      creationCost: response.quote.creation_cost,
      expiresAt: toNumber(response.quote.expires_at),
      grossAmount: response.quote.gross_amount,
      netForwardAmount: response.quote.net_forward_amount,
      payment: mapSpawnPaymentInstructions(response.quote.payment),
      platformFee: response.quote.platform_fee,
      quoteTermsHash: response.quote.quote_terms_hash,
      sessionId: response.quote.session_id
    },
    session: mapSpawnSession(response.session)
  };
}

function mapRegistryPage(
  page: CandidSpawnedAutomatonRegistryPage
): SpawnedAutomatonRegistryPage {
  return {
    items: page.items.map(mapRegistryRecord),
    nextCursor: unwrapOptional(page.next_cursor)
  };
}

function mapListRepositoryStrategiesResponse(
  response: CandidListRepositoryStrategiesResponse
): RepositoryStrategyListResponse {
  return {
    items: response.items.map(mapRepositoryStrategyRecord),
    updatedAt: toNumber(response.updated_at)
  };
}

function mapGetRepositoryStrategyResponse(
  response: CandidGetRepositoryStrategyResponse
): RepositoryStrategyGetResponse {
  return {
    item:
      unwrapOptional(response.item) === null
        ? null
        : mapRepositoryStrategyRecord(
            unwrapOptional(response.item) as CandidRepositoryStrategyRecord
          ),
    updatedAt: toNumber(response.updated_at)
  };
}

function mapFactoryHealth(snapshot: CandidFactoryHealthSnapshot): FactoryHealthSnapshot {
  const awaitingPayment = toNumber(snapshot.active_sessions.awaiting_payment);
  const broadcastingRelease = toNumber(snapshot.active_sessions.broadcasting_release);
  const paymentDetected = toNumber(snapshot.active_sessions.payment_detected);
  const retryableFailed = toNumber(snapshot.active_sessions.retryable_failed);
  const spawning = toNumber(snapshot.active_sessions.spawning);

  return {
    activeSessions: {
      activeTotal:
        awaitingPayment +
        broadcastingRelease +
        paymentDetected +
        retryableFailed +
        spawning,
      awaitingPayment,
      broadcastingRelease,
      paymentDetected,
      retryableFailed,
      spawning
    },
    artifact: {
      loaded: snapshot.artifact.loaded,
      versionCommit: unwrapOptional(snapshot.artifact.version_commit),
      wasmSha256: unwrapOptional(snapshot.artifact.wasm_sha256),
      wasmSizeBytes:
        unwrapOptional(snapshot.artifact.wasm_size_bytes) === null
          ? null
          : toNumber(unwrapOptional(snapshot.artifact.wasm_size_bytes) as bigint)
    },
    currentCanisterBalance: snapshot.current_canister_balance.toString(),
    cyclesPerSpawn: toNumber(snapshot.cycles_per_spawn),
    escrowContractAddress: snapshot.escrow_contract_address,
    estimatedOutcallCyclesPerInterval: toNumber(
      snapshot.estimated_outcall_cycles_per_interval
    ),
    factoryEvmAddress: unwrapOptional(snapshot.factory_evm_address),
    minPoolBalance: toNumber(snapshot.min_pool_balance),
    pause: snapshot.pause
  };
}

export class CanisterFactoryAdapter implements FactoryAdapter {
  private agentPromise?: Promise<HttpAgent>;
  private actorPromise?: Promise<ActorSubclass<FactoryCanisterActor>>;

  constructor(
    private readonly options: {
      canisterId: string;
      host: string;
      createAgent?: (host: string) => Promise<HttpAgent>;
      createActor?: (
        agent: HttpAgent,
        canisterId: string
      ) => Promise<ActorSubclass<FactoryCanisterActor>>;
    }
  ) {}

  async createSpawnSession(
    request: CreateSpawnSessionRequest
  ): Promise<CreateSpawnSessionResponse> {
    const actor = await this.getActor();
    return mapCreateResponse(
      expectOk(await actor.create_spawn_session(mapCreateRequest(request)))
    );
  }

  async getSpawnSession(sessionId: string): Promise<SpawnSessionStatusResponse | null> {
    const actor = await this.getActor();
    const response = await actor.get_spawn_session(sessionId);

    if (isFactoryErrorVariant(response.Err, "SessionNotFound")) {
      return null;
    }

    return mapSessionStatus(expectOk(response));
  }

  async prepareSpawnStewardCommand(command: FactoryStewardCommand): Promise<FactoryStewardProofTemplate> {
    const actor = await this.getActor();
    return mapStewardTemplate(expectOk(await actor.prepare_spawn_steward_command(mapStewardCommand(command))));
  }

  async retrySpawnSession(request: FactoryStewardExecutionRequest): Promise<RetrySpawnResponse> {
    const actor = await this.getActor();
    const result = expectOk(await actor.execute_spawn_steward_command(mapStewardCommand(request.command), mapStewardProof(request)));
    if (!("Retry" in result)) throw new Error("Factory returned a refund result for retry command");
    return { session: mapSessionStatus(result.Retry).session };
  }

  async claimSpawnRefund(request: FactoryStewardExecutionRequest): Promise<RefundSpawnResponse> {
    const actor = await this.getActor();
    const result = expectOk(await actor.execute_spawn_steward_command(mapStewardCommand(request.command), mapStewardProof(request)));
    if (!("Refund" in result)) throw new Error("Factory returned a retry result for refund command");
    return mapRefundResponse(result.Refund);
  }

  async listSpawnedAutomatons(
    cursor: string | undefined,
    limit: number
  ): Promise<SpawnedAutomatonRegistryPage> {
    const actor = await this.getActor();
    return mapRegistryPage(
      expectOk(await actor.list_spawned_automatons(cursor ? [cursor] : [], BigInt(limit)))
    );
  }

  async listRepositoryStrategies(): Promise<RepositoryStrategyListResponse> {
    const actor = await this.getActor();
    return mapListRepositoryStrategiesResponse(
      await actor.list_repository_strategies()
    );
  }

  async getRepositoryStrategy(
    strategyId: string
  ): Promise<RepositoryStrategyGetResponse> {
    const actor = await this.getActor();
    return mapGetRepositoryStrategyResponse(
      await actor.get_repository_strategy(strategyId)
    );
  }

  async listRoomMessages(
    afterSeq: number | undefined,
    limit: number
  ): Promise<RoomMessagePage> {
    const actor = await this.getActor();
    return mapRoomMessagePage(
      expectOk(
        await actor.list_room_messages(
          afterSeq === undefined ? [] : [BigInt(afterSeq)],
          [BigInt(limit)]
        )
      )
    );
  }

  async listMessagesForAutomaton(
    canisterId: string,
    afterSeq: number | undefined,
    limit: number
  ): Promise<RoomMessagePage> {
    const actor = await this.getActor();
    return mapRoomMessagePage(
      expectOk(
        await actor.list_messages_for_automaton(
          canisterId,
          afterSeq === undefined ? [] : [BigInt(afterSeq)],
          [BigInt(limit)]
        )
      )
    );
  }

  async getSpawnedAutomaton(canisterId: string): Promise<SpawnedAutomatonRecord | null> {
    const actor = await this.getActor();
    const response = await actor.get_spawned_automaton(canisterId);

    if (isFactoryErrorVariant(response.Err, "RegistryRecordNotFound")) {
      return null;
    }

    return mapRegistryRecord(expectOk(response));
  }

  async getFactoryHealth(): Promise<FactoryHealthSnapshot> {
    const actor = await this.getActor();
    return mapFactoryHealth(await actor.get_factory_health());
  }

  private async getAgent() {
    this.agentPromise ??= (async () => {
      const agent =
        this.options.createAgent !== undefined
          ? await this.options.createAgent(this.options.host)
          : await HttpAgent.create({
              host: this.options.host
            });

      if (!this.options.host.startsWith("https://")) {
        await agent.fetchRootKey();
      }

      return agent;
    })();

    return this.agentPromise;
  }

  private async getActor() {
    this.actorPromise ??= (async () => {
      const agent = await this.getAgent();

      if (this.options.createActor !== undefined) {
        return this.options.createActor(agent, this.options.canisterId);
      }

      return Actor.createActor<FactoryCanisterActor>(
        createFactoryIdl() as unknown as Parameters<typeof Actor.createActor>[0],
        {
          agent,
          canisterId: this.options.canisterId
        }
      );
    })();

    return this.actorPromise;
  }
}
