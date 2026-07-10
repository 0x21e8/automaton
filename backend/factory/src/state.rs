use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};

use candid::CandidType;
use ic_stable_structures::{
    memory_manager::{MemoryId, MemoryManager, VirtualMemory},
    storable::Bound,
    Cell as StableCell, DefaultMemoryImpl, Memory, StableBTreeMap, Storable,
};
use serde::{Deserialize, Serialize};

use crate::types::{
    AutomatonChildRuntimeConfig, AutomatonRuntimeState, CreationCostQuote, EscrowClaim,
    FactoryError, FactoryInitArgs, FeeConfig, PendingArtifactUpload, ReleaseBroadcastConfig,
    RepositoryStrategyRecord, RoomMessage, RoomState, SchedulerJob, SchedulerRuntime,
    SessionAuditActor, SessionAuditEntry, SpawnProviderSecrets, SpawnSession, SpawnSessionState,
    SpawnedAutomatonRecord,
};

const STORAGE_SCHEMA_VERSION: u32 = 5;
const PREVIOUS_STORAGE_SCHEMA_VERSION: u32 = 4;
const STORAGE_METADATA_MEMORY_ID: u8 = 0;
const FACTORY_CONFIG_MEMORY_ID: u8 = 1;
const SESSIONS_MEMORY_ID: u8 = 2;
const ESCROW_CLAIMS_MEMORY_ID: u8 = 3;
const REGISTRY_MEMORY_ID: u8 = 4;
const RUNTIMES_MEMORY_ID: u8 = 5;
const AUDIT_LOG_MEMORY_ID: u8 = 6;
const SCHEDULER_RUNTIME_MEMORY_ID: u8 = 7;
const SCHEDULER_JOBS_MEMORY_ID: u8 = 8;
const ROOM_STATE_MEMORY_ID: u8 = 9;
const ROOM_MESSAGES_MEMORY_ID: u8 = 10;
const STRATEGY_REPOSITORY_MEMORY_ID: u8 = 11;
const SPAWN_PROVIDER_SECRETS_MEMORY_ID: u8 = 12;

type StableMemory<M> = VirtualMemory<M>;

macro_rules! impl_candid_storable {
    ($ty:ty) => {
        impl Storable for $ty {
            fn to_bytes(&self) -> Cow<'_, [u8]> {
                Cow::Owned(candid::encode_one(self.clone()).expect("candid encoding should work"))
            }

            fn into_bytes(self) -> Vec<u8> {
                candid::encode_one(self).expect("candid encoding should work")
            }

            fn from_bytes(bytes: Cow<[u8]>) -> Self {
                candid::decode_one(bytes.as_ref()).expect("candid decoding should work")
            }

            const BOUND: Bound = Bound::Unbounded;
        }
    };
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
pub struct FactoryState {
    pub repository_strategies: BTreeMap<String, RepositoryStrategyRecord>,
    pub sessions: BTreeMap<String, SpawnSession>,
    pub escrow_claims: BTreeMap<String, EscrowClaim>,
    pub registry: BTreeMap<String, SpawnedAutomatonRecord>,
    pub runtimes: BTreeMap<String, AutomatonRuntimeState>,
    pub audit_log: BTreeMap<String, Vec<SessionAuditEntry>>,
    pub room_state: RoomState,
    pub room_messages: BTreeMap<u64, RoomMessage>,
    pub scheduler_jobs: BTreeMap<String, SchedulerJob>,
    pub scheduler_runtime: SchedulerRuntime,
    pub fee_config: FeeConfig,
    pub creation_cost_quote: CreationCostQuote,
    pub release_broadcast_config: ReleaseBroadcastConfig,
    pub child_runtime: AutomatonChildRuntimeConfig,
    pub pause: bool,
    pub next_session_nonce: u64,
    pub next_automaton_nonce: u64,
    pub payment_address: String,
    pub escrow_contract_address: String,
    pub payment_last_scanned_block: Option<u64>,
    pub next_payment_poll_at_ms: Option<u64>,
    pub factory_evm_address: Option<String>,
    pub factory_evm_address_derived_at: Option<u64>,
    pub base_rpc_endpoint: Option<String>,
    pub base_rpc_fallback_endpoint: Option<String>,
    pub cycles_per_spawn: u64,
    pub min_pool_balance: u64,
    pub estimated_outcall_cycles_per_interval: u64,
    pub wasm_bytes: Option<Vec<u8>>,
    pub wasm_sha256: Option<String>,
    pub pending_artifact_upload: Option<PendingArtifactUpload>,
    pub admin_principals: BTreeSet<String>,
    pub session_ttl_ms: u64,
    pub version_commit: String,
}

pub type FactoryStateSnapshot = FactoryState;

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
enum AuditStorageLayout {
    SessionKeyedCollection,
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
struct StorageMetadata {
    schema_version: u32,
    audit_storage_layout: AuditStorageLayout,
}

impl Default for StorageMetadata {
    fn default() -> Self {
        Self {
            schema_version: STORAGE_SCHEMA_VERSION,
            audit_storage_layout: AuditStorageLayout::SessionKeyedCollection,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, CandidType, Serialize, Deserialize)]
struct FactoryStableConfig {
    fee_config: FeeConfig,
    creation_cost_quote: CreationCostQuote,
    release_broadcast_config: ReleaseBroadcastConfig,
    child_runtime: Option<AutomatonChildRuntimeConfig>,
    pause: bool,
    next_session_nonce: u64,
    next_automaton_nonce: u64,
    payment_address: String,
    escrow_contract_address: String,
    payment_last_scanned_block: Option<u64>,
    next_payment_poll_at_ms: Option<u64>,
    factory_evm_address: Option<String>,
    factory_evm_address_derived_at: Option<u64>,
    base_rpc_endpoint: Option<String>,
    base_rpc_fallback_endpoint: Option<String>,
    cycles_per_spawn: u64,
    min_pool_balance: u64,
    estimated_outcall_cycles_per_interval: u64,
    wasm_bytes: Option<Vec<u8>>,
    wasm_sha256: Option<String>,
    pending_artifact_upload: Option<PendingArtifactUpload>,
    admin_principals: BTreeSet<String>,
    session_ttl_ms: u64,
    version_commit: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Default, CandidType, Serialize, Deserialize)]
struct StableAuditEntries {
    entries: Vec<SessionAuditEntry>,
}

impl Default for FactoryState {
    fn default() -> Self {
        let mut admin_principals = BTreeSet::new();
        admin_principals.insert("admin".to_string());

        Self {
            repository_strategies: crate::strategy_repository::seed_repository_records(0),
            sessions: BTreeMap::new(),
            escrow_claims: BTreeMap::new(),
            registry: BTreeMap::new(),
            runtimes: BTreeMap::new(),
            audit_log: BTreeMap::new(),
            room_state: RoomState::default(),
            room_messages: BTreeMap::new(),
            scheduler_jobs: BTreeMap::new(),
            scheduler_runtime: SchedulerRuntime::default(),
            fee_config: FeeConfig {
                usdc_fee: "5000000".to_string(),
                updated_at: 0,
            },
            creation_cost_quote: CreationCostQuote {
                usdc_cost: "45000000".to_string(),
                updated_at: 0,
            },
            release_broadcast_config: ReleaseBroadcastConfig::default(),
            child_runtime: AutomatonChildRuntimeConfig::default(),
            pause: false,
            next_session_nonce: 0,
            next_automaton_nonce: 0,
            payment_address: "0x1111111111111111111111111111111111111111".to_string(),
            escrow_contract_address: "0x2222222222222222222222222222222222222222".to_string(),
            payment_last_scanned_block: None,
            next_payment_poll_at_ms: None,
            factory_evm_address: None,
            factory_evm_address_derived_at: None,
            base_rpc_endpoint: None,
            base_rpc_fallback_endpoint: None,
            cycles_per_spawn: 0,
            min_pool_balance: 0,
            estimated_outcall_cycles_per_interval: 0,
            wasm_bytes: None,
            wasm_sha256: None,
            pending_artifact_upload: None,
            admin_principals,
            session_ttl_ms: 30 * 60 * 1_000,
            version_commit: "dev-build".to_string(),
        }
    }
}

impl Default for FactoryStableConfig {
    fn default() -> Self {
        Self::from(&FactoryState::default())
    }
}

impl From<&FactoryState> for FactoryStableConfig {
    fn from(value: &FactoryState) -> Self {
        Self {
            fee_config: value.fee_config.clone(),
            creation_cost_quote: value.creation_cost_quote.clone(),
            release_broadcast_config: value.release_broadcast_config.clone(),
            child_runtime: Some(value.child_runtime.clone()),
            pause: value.pause,
            next_session_nonce: value.next_session_nonce,
            next_automaton_nonce: value.next_automaton_nonce,
            payment_address: value.payment_address.clone(),
            escrow_contract_address: value.escrow_contract_address.clone(),
            payment_last_scanned_block: value.payment_last_scanned_block,
            next_payment_poll_at_ms: value.next_payment_poll_at_ms,
            factory_evm_address: value.factory_evm_address.clone(),
            factory_evm_address_derived_at: value.factory_evm_address_derived_at,
            base_rpc_endpoint: value.base_rpc_endpoint.clone(),
            base_rpc_fallback_endpoint: value.base_rpc_fallback_endpoint.clone(),
            cycles_per_spawn: value.cycles_per_spawn,
            min_pool_balance: value.min_pool_balance,
            estimated_outcall_cycles_per_interval: value.estimated_outcall_cycles_per_interval,
            wasm_bytes: value.wasm_bytes.clone(),
            wasm_sha256: value.wasm_sha256.clone(),
            pending_artifact_upload: value.pending_artifact_upload.clone(),
            admin_principals: value.admin_principals.clone(),
            session_ttl_ms: value.session_ttl_ms,
            version_commit: value.version_commit.clone(),
        }
    }
}

impl FactoryStableConfig {
    #[allow(clippy::too_many_arguments)]
    fn into_state(
        self,
        repository_strategies: BTreeMap<String, RepositoryStrategyRecord>,
        sessions: BTreeMap<String, SpawnSession>,
        escrow_claims: BTreeMap<String, EscrowClaim>,
        registry: BTreeMap<String, SpawnedAutomatonRecord>,
        runtimes: BTreeMap<String, AutomatonRuntimeState>,
        room_state: RoomState,
        room_messages: BTreeMap<u64, RoomMessage>,
        scheduler_jobs: BTreeMap<String, SchedulerJob>,
        scheduler_runtime: SchedulerRuntime,
        audit_log: BTreeMap<String, Vec<SessionAuditEntry>>,
    ) -> FactoryState {
        FactoryState {
            repository_strategies,
            sessions,
            escrow_claims,
            registry,
            runtimes,
            room_state,
            room_messages,
            scheduler_jobs,
            scheduler_runtime,
            audit_log,
            fee_config: self.fee_config,
            creation_cost_quote: self.creation_cost_quote,
            release_broadcast_config: self.release_broadcast_config,
            child_runtime: self.child_runtime.unwrap_or_default(),
            pause: self.pause,
            next_session_nonce: self.next_session_nonce,
            next_automaton_nonce: self.next_automaton_nonce,
            payment_address: self.payment_address,
            escrow_contract_address: self.escrow_contract_address,
            payment_last_scanned_block: self.payment_last_scanned_block,
            next_payment_poll_at_ms: self.next_payment_poll_at_ms,
            factory_evm_address: self.factory_evm_address,
            factory_evm_address_derived_at: self.factory_evm_address_derived_at,
            base_rpc_endpoint: self.base_rpc_endpoint,
            base_rpc_fallback_endpoint: self.base_rpc_fallback_endpoint,
            cycles_per_spawn: self.cycles_per_spawn,
            min_pool_balance: self.min_pool_balance,
            estimated_outcall_cycles_per_interval: self.estimated_outcall_cycles_per_interval,
            wasm_bytes: self.wasm_bytes,
            wasm_sha256: self.wasm_sha256,
            pending_artifact_upload: self.pending_artifact_upload,
            admin_principals: self.admin_principals,
            session_ttl_ms: self.session_ttl_ms,
            version_commit: self.version_commit,
        }
    }
}

impl_candid_storable!(SpawnSession);
impl_candid_storable!(SpawnProviderSecrets);
impl_candid_storable!(EscrowClaim);
impl_candid_storable!(SpawnedAutomatonRecord);
impl_candid_storable!(AutomatonRuntimeState);
impl_candid_storable!(RepositoryStrategyRecord);
impl_candid_storable!(StorageMetadata);
impl_candid_storable!(FactoryStableConfig);
impl_candid_storable!(StableAuditEntries);
impl_candid_storable!(RoomState);
impl_candid_storable!(RoomMessage);
impl_candid_storable!(SchedulerRuntime);
impl_candid_storable!(SchedulerJob);

struct FactoryStableStorage<M: Memory> {
    _memory_manager: MemoryManager<M>,
    metadata: StableCell<StorageMetadata, StableMemory<M>>,
    config: StableCell<FactoryStableConfig, StableMemory<M>>,
    scheduler_runtime: StableCell<SchedulerRuntime, StableMemory<M>>,
    room_state: StableCell<RoomState, StableMemory<M>>,
    repository_strategies: StableBTreeMap<String, RepositoryStrategyRecord, StableMemory<M>>,
    sessions: StableBTreeMap<String, SpawnSession, StableMemory<M>>,
    spawn_provider_secrets: StableBTreeMap<String, SpawnProviderSecrets, StableMemory<M>>,
    escrow_claims: StableBTreeMap<String, EscrowClaim, StableMemory<M>>,
    registry: StableBTreeMap<String, SpawnedAutomatonRecord, StableMemory<M>>,
    runtimes: StableBTreeMap<String, AutomatonRuntimeState, StableMemory<M>>,
    audit_log: StableBTreeMap<String, StableAuditEntries, StableMemory<M>>,
    room_messages: StableBTreeMap<u64, RoomMessage, StableMemory<M>>,
    scheduler_jobs: StableBTreeMap<String, SchedulerJob, StableMemory<M>>,
}

impl<M: Memory> FactoryStableStorage<M> {
    fn open(memory: M) -> Self {
        Self::init(memory)
    }

    fn init(memory: M) -> Self {
        let memory_manager = MemoryManager::init(memory);
        let metadata = StableCell::init(
            memory_manager.get(MemoryId::new(STORAGE_METADATA_MEMORY_ID)),
            StorageMetadata::default(),
        );
        let config = StableCell::init(
            memory_manager.get(MemoryId::new(FACTORY_CONFIG_MEMORY_ID)),
            FactoryStableConfig::default(),
        );
        let scheduler_runtime = StableCell::init(
            memory_manager.get(MemoryId::new(SCHEDULER_RUNTIME_MEMORY_ID)),
            SchedulerRuntime::default(),
        );
        let room_state = StableCell::init(
            memory_manager.get(MemoryId::new(ROOM_STATE_MEMORY_ID)),
            RoomState::default(),
        );

        Self {
            metadata,
            config,
            scheduler_runtime,
            room_state,
            repository_strategies: StableBTreeMap::init(
                memory_manager.get(MemoryId::new(STRATEGY_REPOSITORY_MEMORY_ID)),
            ),
            sessions: StableBTreeMap::init(memory_manager.get(MemoryId::new(SESSIONS_MEMORY_ID))),
            spawn_provider_secrets: StableBTreeMap::init(
                memory_manager.get(MemoryId::new(SPAWN_PROVIDER_SECRETS_MEMORY_ID)),
            ),
            escrow_claims: StableBTreeMap::init(
                memory_manager.get(MemoryId::new(ESCROW_CLAIMS_MEMORY_ID)),
            ),
            registry: StableBTreeMap::init(memory_manager.get(MemoryId::new(REGISTRY_MEMORY_ID))),
            runtimes: StableBTreeMap::init(memory_manager.get(MemoryId::new(RUNTIMES_MEMORY_ID))),
            audit_log: StableBTreeMap::init(memory_manager.get(MemoryId::new(AUDIT_LOG_MEMORY_ID))),
            room_messages: StableBTreeMap::init(
                memory_manager.get(MemoryId::new(ROOM_MESSAGES_MEMORY_ID)),
            ),
            scheduler_jobs: StableBTreeMap::init(
                memory_manager.get(MemoryId::new(SCHEDULER_JOBS_MEMORY_ID)),
            ),
            _memory_manager: memory_manager,
        }
    }

    fn assert_supported_schema(&self) {
        let metadata = self.metadata.get();
        assert!(
            metadata.schema_version == STORAGE_SCHEMA_VERSION
                || metadata.schema_version == PREVIOUS_STORAGE_SCHEMA_VERSION,
            "unsupported factory storage schema version: {}",
            metadata.schema_version
        );
        assert_eq!(
            metadata.audit_storage_layout,
            AuditStorageLayout::SessionKeyedCollection,
            "unsupported factory audit log layout"
        );
    }

    fn load_state(&self) -> FactoryState {
        self.assert_supported_schema();

        self.config.get().clone().into_state(
            load_collection(&self.repository_strategies),
            load_collection(&self.sessions),
            load_collection(&self.escrow_claims),
            load_collection(&self.registry),
            load_collection(&self.runtimes),
            self.room_state.get().clone(),
            load_collection(&self.room_messages),
            load_collection(&self.scheduler_jobs),
            self.scheduler_runtime.get().clone(),
            self.audit_log
                .iter()
                .map(|entry| {
                    let (session_id, entries) = entry.into_pair();
                    (session_id, entries.entries)
                })
                .collect(),
        )
    }

    fn replace_state(&mut self, next: &FactoryState) {
        let current = self.load_state();
        self.persist_delta(&current, next);
    }

    fn mutate<T>(&mut self, writer: impl FnOnce(&mut FactoryState) -> T) -> T {
        let mut next = self.load_state();
        let current = next.clone();
        let result = writer(&mut next);
        self.persist_delta(&current, &next);
        result
    }

    fn persist_delta(&mut self, current: &FactoryState, next: &FactoryState) {
        let expected_metadata = StorageMetadata::default();
        if self.metadata.get() != &expected_metadata {
            self.metadata.set(expected_metadata);
        }

        let current_config = FactoryStableConfig::from(current);
        let next_config = FactoryStableConfig::from(next);
        if current_config != next_config {
            self.config.set(next_config);
        }
        if self.scheduler_runtime.get() != &next.scheduler_runtime {
            self.scheduler_runtime.set(next.scheduler_runtime.clone());
        }
        if self.room_state.get() != &next.room_state {
            self.room_state.set(next.room_state.clone());
        }

        sync_collection(
            &mut self.repository_strategies,
            &current.repository_strategies,
            &next.repository_strategies,
        );
        sync_collection(&mut self.sessions, &current.sessions, &next.sessions);
        sync_collection(
            &mut self.escrow_claims,
            &current.escrow_claims,
            &next.escrow_claims,
        );
        sync_collection(&mut self.registry, &current.registry, &next.registry);
        sync_collection(&mut self.runtimes, &current.runtimes, &next.runtimes);
        sync_collection(
            &mut self.room_messages,
            &current.room_messages,
            &next.room_messages,
        );
        sync_collection(
            &mut self.scheduler_jobs,
            &current.scheduler_jobs,
            &next.scheduler_jobs,
        );
        sync_audit_log(&mut self.audit_log, &current.audit_log, &next.audit_log);
    }
}

fn load_collection<K, V, M>(map: &StableBTreeMap<K, V, StableMemory<M>>) -> BTreeMap<K, V>
where
    K: Clone + Ord + Storable,
    V: Storable,
    M: Memory,
{
    map.iter().map(|entry| entry.into_pair()).collect()
}

fn sync_collection<K, V, M>(
    map: &mut StableBTreeMap<K, V, StableMemory<M>>,
    current: &BTreeMap<K, V>,
    next: &BTreeMap<K, V>,
) where
    K: Clone + Ord + Storable,
    V: Clone + Eq + Storable,
    M: Memory,
{
    for key in current.keys() {
        if !next.contains_key(key) {
            map.remove(key);
        }
    }

    for (key, value) in next {
        if current.get(key) != Some(value) {
            map.insert(key.clone(), value.clone());
        }
    }
}

fn sync_audit_log<M: Memory>(
    map: &mut StableBTreeMap<String, StableAuditEntries, StableMemory<M>>,
    current: &BTreeMap<String, Vec<SessionAuditEntry>>,
    next: &BTreeMap<String, Vec<SessionAuditEntry>>,
) {
    for session_id in current.keys() {
        if !next.contains_key(session_id) {
            map.remove(session_id);
        }
    }

    for (session_id, entries) in next {
        if current.get(session_id) != Some(entries) {
            map.insert(
                session_id.clone(),
                StableAuditEntries {
                    entries: entries.clone(),
                },
            );
        }
    }
}

#[cfg(target_arch = "wasm32")]
thread_local! {
    static FACTORY_STORAGE: RefCell<FactoryStableStorage<DefaultMemoryImpl>> = RefCell::new(
        FactoryStableStorage::open(DefaultMemoryImpl::default())
    );
}

#[cfg(not(target_arch = "wasm32"))]
struct TestStorageHandle {
    storage: FactoryStableStorage<DefaultMemoryImpl>,
    #[cfg(test)]
    memory: DefaultMemoryImpl,
}

#[cfg(not(target_arch = "wasm32"))]
impl TestStorageHandle {
    fn new() -> Self {
        let memory = DefaultMemoryImpl::default();
        let storage = FactoryStableStorage::open(memory.clone());
        Self {
            storage,
            #[cfg(test)]
            memory,
        }
    }

    #[cfg(test)]
    fn reload(&mut self) {
        self.storage = FactoryStableStorage::open(self.memory.clone());
    }
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static FACTORY_STORAGE: RefCell<TestStorageHandle> = RefCell::new(TestStorageHandle::new());
}

#[cfg(not(target_arch = "wasm32"))]
thread_local! {
    static MOCK_CANISTER_BALANCE: RefCell<u128> = const { RefCell::new(u128::MAX) };
}

#[cfg(target_arch = "wasm32")]
fn with_storage<T>(reader: impl FnOnce(&FactoryStableStorage<DefaultMemoryImpl>) -> T) -> T {
    FACTORY_STORAGE.with(|storage| reader(&storage.borrow()))
}

#[cfg(not(target_arch = "wasm32"))]
fn with_storage<T>(reader: impl FnOnce(&FactoryStableStorage<DefaultMemoryImpl>) -> T) -> T {
    FACTORY_STORAGE.with(|storage| reader(&storage.borrow().storage))
}

#[cfg(target_arch = "wasm32")]
fn with_storage_mut<T>(
    writer: impl FnOnce(&mut FactoryStableStorage<DefaultMemoryImpl>) -> T,
) -> T {
    FACTORY_STORAGE.with(|storage| writer(&mut storage.borrow_mut()))
}

#[cfg(not(target_arch = "wasm32"))]
fn with_storage_mut<T>(
    writer: impl FnOnce(&mut FactoryStableStorage<DefaultMemoryImpl>) -> T,
) -> T {
    FACTORY_STORAGE.with(|storage| writer(&mut storage.borrow_mut().storage))
}

pub fn read_state<T>(reader: impl FnOnce(&FactoryState) -> T) -> T {
    with_storage(|storage| {
        let state = storage.load_state();
        reader(&state)
    })
}

pub fn write_state<T>(writer: impl FnOnce(&mut FactoryState) -> T) -> T {
    with_storage_mut(|storage| storage.mutate(writer))
}

pub fn snapshot_state() -> FactoryStateSnapshot {
    read_state(|state| state.clone())
}

pub fn restore_state(snapshot: FactoryStateSnapshot) {
    with_storage_mut(|storage| storage.replace_state(&snapshot));
}

pub fn store_spawn_provider_secrets(session_id: &str, secrets: SpawnProviderSecrets) {
    with_storage_mut(|storage| {
        storage
            .spawn_provider_secrets
            .insert(session_id.to_string(), secrets);
    });
}

pub fn load_spawn_provider_secrets(session_id: &str) -> Option<SpawnProviderSecrets> {
    with_storage(|storage| storage.spawn_provider_secrets.get(&session_id.to_string()))
}

pub fn delete_spawn_provider_secrets(session_id: &str) {
    with_storage_mut(|storage| {
        storage
            .spawn_provider_secrets
            .remove(&session_id.to_string());
    });
}

#[cfg(target_arch = "wasm32")]
pub fn initialize_storage_after_upgrade() {
    let _ = read_state(|state| state.version_commit.clone());
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub(crate) fn reload_storage_for_test() {
    #[cfg(not(target_arch = "wasm32"))]
    FACTORY_STORAGE.with(|storage| storage.borrow_mut().reload());
}

#[cfg(target_arch = "wasm32")]
pub fn current_canister_balance() -> u128 {
    ic_cdk::api::canister_cycle_balance()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn current_canister_balance() -> u128 {
    MOCK_CANISTER_BALANCE.with(|balance| *balance.borrow())
}

#[cfg(all(test, not(target_arch = "wasm32")))]
pub fn set_mock_canister_balance(balance: u128) {
    MOCK_CANISTER_BALANCE.with(|value| {
        *value.borrow_mut() = balance;
    });
}

pub fn apply_factory_init_args(args: FactoryInitArgs, init_caller: Option<String>) -> FactoryState {
    let mut state = FactoryState::default();

    if let Some(fee_config) = args.fee_config {
        state.fee_config = fee_config;
    }

    if let Some(creation_cost_quote) = args.creation_cost_quote {
        state.creation_cost_quote = creation_cost_quote;
    }

    if let Some(release_broadcast_config) = args.release_broadcast_config {
        state.release_broadcast_config = release_broadcast_config;
    }
    if let Some(child_runtime) = args.child_runtime {
        state.child_runtime = child_runtime;
        if let Some(chain_id) = state.child_runtime.evm_chain_id {
            state.release_broadcast_config.chain_id = chain_id;
        }
    }

    state.pause = args.pause;

    if let Some(payment_address) = args.payment_address {
        state.payment_address = payment_address;
    }

    if let Some(escrow_contract_address) = args.escrow_contract_address {
        state.escrow_contract_address = escrow_contract_address;
    }

    state.base_rpc_endpoint = args.base_rpc_endpoint;
    state.base_rpc_fallback_endpoint = args.base_rpc_fallback_endpoint;
    state.cycles_per_spawn = args.cycles_per_spawn.unwrap_or(0);
    state.min_pool_balance = args.min_pool_balance.unwrap_or(0);
    state.estimated_outcall_cycles_per_interval =
        args.estimated_outcall_cycles_per_interval.unwrap_or(0);
    state.session_ttl_ms = args.session_ttl_ms.unwrap_or(state.session_ttl_ms);
    state.version_commit = args.version_commit.unwrap_or(state.version_commit);
    state.wasm_sha256 = args.wasm_sha256;

    if let Some(admin_caller) = init_caller {
        if args.admin_principals.is_empty() {
            state.admin_principals.clear();
            state.admin_principals.insert(admin_caller);
        } else {
            state.admin_principals = args
                .admin_principals
                .into_iter()
                .map(|principal| principal.to_text())
                .collect();
        }
    } else if !args.admin_principals.is_empty() {
        state.admin_principals = args
            .admin_principals
            .into_iter()
            .map(|principal| principal.to_text())
            .collect();
    }

    state
}

pub fn ensure_admin_in_state(state: &FactoryState, caller: &str) -> Result<(), FactoryError> {
    if state.admin_principals.contains(caller) {
        return Ok(());
    }
    Err(FactoryError::UnauthorizedAdmin {
        caller: caller.to_string(),
    })
}

pub fn insert_spawned_automaton_record(record: SpawnedAutomatonRecord) {
    write_state(|state| {
        state.registry.insert(record.canister_id.clone(), record);
    });
}

pub fn clear_provider_secrets(
    _session: &SpawnSession,
    runtime: Option<&mut AutomatonRuntimeState>,
) {
    if let Some(runtime) = runtime {
        runtime.provider_keys_cleared = true;
    }
}

const MAX_AUDIT_ENTRIES_PER_SESSION: usize = 100;

pub fn record_session_audit(
    state: &mut FactoryState,
    session_id: &str,
    from_state: Option<SpawnSessionState>,
    to_state: SpawnSessionState,
    actor: SessionAuditActor,
    timestamp: u64,
    reason: &str,
) {
    let entries = state.audit_log.entry(session_id.to_string()).or_default();
    entries.push(SessionAuditEntry {
        session_id: session_id.to_string(),
        timestamp,
        from_state,
        to_state,
        actor,
        reason: reason.to_string(),
    });
    if entries.len() > MAX_AUDIT_ENTRIES_PER_SESSION {
        let excess = entries.len() - MAX_AUDIT_ENTRIES_PER_SESSION;
        entries.drain(0..excess);
    }
}
