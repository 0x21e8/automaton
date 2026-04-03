const SUBMIT_ROUTE = "/v1/strategy-discovery/jobs";
const HEALTH_ROUTE = "/health";
const CALLBACK_METHOD = "submit_strategy_discovery_result";
const MAX_REQUEST_BYTES = 128 * 1024;
const DEFAULT_IC_HOST = "https://icp-api.io";
const DEFAULT_ABI_FETCH_DELAY_MS = 350;

function json(status, payload) {
  return new Response(JSON.stringify(payload), {
    status,
    headers: {
      "content-type": "application/json; charset=utf-8",
      "cache-control": "no-store",
    },
  });
}

function toErrorMessage(error) {
  if (error instanceof Error) {
    return error.message;
  }
  return String(error);
}

function nowNs() {
  return BigInt(Date.now()) * 1_000_000n;
}

function nowAckNs() {
  return Date.now() * 1_000;
}

function normalizeHost(url) {
  if (typeof url !== "string" || !url.includes("://")) {
    return null;
  }
  try {
    return new URL(url).host.toLowerCase();
  } catch {
    return null;
  }
}

function curatedSourceHosts(env) {
  return new Set(
    String(env.CURATED_SOURCE_HOSTS || "")
      .split(",")
      .map((entry) => entry.trim().toLowerCase())
      .filter(Boolean),
  );
}

function requireBearerAuth(request, env) {
  const expected = String(env.STRATEGY_DISCOVERY_WORKER_API_KEY || "").trim();
  if (!expected) {
    throw new Error("STRATEGY_DISCOVERY_WORKER_API_KEY is not configured");
  }
  const value = request.headers.get("authorization");
  if (!value || !value.startsWith("Bearer ")) {
    throw new Error("missing bearer authorization");
  }
  if (value.slice("Bearer ".length).trim() !== expected) {
    throw new Error("invalid bearer authorization");
  }
}

function validateWatchlistEntry(entry, env) {
  if (!entry || typeof entry !== "object") {
    throw new Error("watchlist entry must be an object");
  }
  const id = String(entry.id || "").trim();
  const marketDataApiUrl = String(entry.market_data_api_url || "").trim();
  const abiApiUrl = String(entry.abi_api_url || "").trim();
  if (!id) {
    throw new Error("watchlist entry id is required");
  }
  if (!entry.chain_id) {
    throw new Error(`watchlist entry ${id} chain_id is required`);
  }
  if (!String(entry.pool_address || "").trim()) {
    throw new Error(`watchlist entry ${id} pool_address is required`);
  }
  if (!marketDataApiUrl || !abiApiUrl) {
    throw new Error(`watchlist entry ${id} api urls are required`);
  }

  const allowedHosts = curatedSourceHosts(env);
  for (const url of [marketDataApiUrl, abiApiUrl]) {
    const host = normalizeHost(url);
    if (!host) {
      throw new Error(`watchlist entry ${id} invalid source url ${url}`);
    }
    if (allowedHosts.size > 0 && !allowedHosts.has(host)) {
      throw new Error(`watchlist entry ${id} host ${host} not in curated allowlist`);
    }
  }

  return {
    id,
    chain_id: Number(entry.chain_id),
    pool_address: String(entry.pool_address).trim().toLowerCase(),
    market_data_api_url: marketDataApiUrl,
    abi_api_url: abiApiUrl,
  };
}

export function parseStrategyDiscoveryJob(body, env) {
  if (!body || typeof body !== "object") {
    throw new Error("job payload must be a JSON object");
  }
  const canisterId = String(body.canister_id || "").trim();
  const jobId = String(body.job_id || "").trim();
  const objective = String(body.objective || "").trim();
  const watchlist = Array.isArray(body.watchlist)
    ? body.watchlist.map((entry) => validateWatchlistEntry(entry, env))
    : [];
  if (!canisterId) {
    throw new Error("canister_id is required");
  }
  if (!jobId) {
    throw new Error("job_id is required");
  }
  if (!objective) {
    throw new Error("objective is required");
  }
  if (watchlist.length === 0) {
    throw new Error("watchlist must contain at least one entry");
  }
  return {
    canister_id: canisterId,
    job_id: jobId,
    objective,
    watchlist,
    exposure_summary: String(body.exposure_summary || "").trim(),
    autonomy_summary: String(body.autonomy_summary || "").trim(),
    freshness_constraints:
      body.freshness_constraints && typeof body.freshness_constraints === "object"
        ? body.freshness_constraints
        : {},
  };
}

async function delay(ms) {
  await new Promise((resolve) => setTimeout(resolve, ms));
}

async function fetchJson(url) {
  const response = await fetch(url, {
    headers: {
      accept: "application/json",
    },
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`fetch ${url} failed with ${response.status}: ${text.slice(0, 200)}`);
  }
  try {
    return JSON.parse(text);
  } catch (error) {
    throw new Error(`fetch ${url} returned invalid json: ${toErrorMessage(error)}`);
  }
}

function normalizeAbiJson(value) {
  if (Array.isArray(value)) {
    return JSON.stringify(value);
  }
  if (value && typeof value === "object") {
    if (typeof value.result === "string") {
      return value.result;
    }
    if (Array.isArray(value.abi)) {
      return JSON.stringify(value.abi);
    }
  }
  throw new Error("abi response missing ABI payload");
}

function buildSourceRecord(sourceId, sourceType, url) {
  return {
    source_id: sourceId,
    source_type: sourceType,
    url,
    fetched_at_ns: nowNs(),
    content_hash: `hash:${sourceId}`,
    trust_tier: "Official",
  };
}

function extractMetric(json, keys) {
  for (const key of keys) {
    if (json && typeof json === "object" && json[key] !== undefined && json[key] !== null) {
      return json[key];
    }
  }
  return null;
}

function buildProtocolMarketSnapshot(entry, marketJson) {
  const tvl = extractMetric(marketJson, ["tvlUsd", "tvl", "totalLiquidityUsd"]);
  const supplyApy = extractMetric(marketJson, ["supplyApyBps", "supplyApy", "apyBase"]);
  const borrowApy = extractMetric(marketJson, ["borrowApyBps", "borrowApy"]);
  const utilization = extractMetric(marketJson, ["utilizationBps", "utilization"]);
  return {
    protocol_id: entry.id,
    chain_id: entry.chain_id,
    pool_address: entry.pool_address,
    tvl_usd: tvl == null ? [] : [String(tvl)],
    supply_apy_bps: supplyApy == null ? [] : [Number(Math.round(Number(supplyApy)))],
    borrow_apy_bps: borrowApy == null ? [] : [Number(Math.round(Number(borrowApy)))],
    utilization_bps: utilization == null ? [] : [Number(Math.round(Number(utilization)))],
    summary: `market snapshot for ${entry.id}`,
    warnings: [],
  };
}

function buildProtocolArtifactBundle(entry, abiJson) {
  return {
    bundle_id: `${entry.id}:pool`,
    protocol_id: entry.id,
    chain_id: entry.chain_id,
    role: "pool",
    contract_address: entry.pool_address,
    abi_json: abiJson,
    source_ref: entry.abi_api_url,
    codehash: [],
    selector_assertions: [],
    spec_summary: `worker-collected ABI for ${entry.id} pool`,
    risk_notes: [],
  };
}

function buildStrategyCandidate(entry, marketSnapshot, objective) {
  const estimatedYield = marketSnapshot.supply_apy_bps?.[0];
  return {
    candidate_id: `${entry.id}:reserve_supply`,
    objective,
    protocol_id: entry.id,
    primitive: "reserve_supply",
    chain_id: entry.chain_id,
    rationale: `deterministic candidate derived from ${entry.id} market and ABI inputs`,
    required_artifacts: [`${entry.id}:pool`],
    assumptions: ["market snapshot remains representative until refreshed"],
    missing_inputs: [],
    confidence_label: estimatedYield == null ? "low" : "medium",
    freshness_deadline_ns: [],
    suggested_template_shape: [`base-${entry.id}-reserve`],
    estimated_yield_bps: estimatedYield == null ? [] : [estimatedYield],
    warnings: [],
  };
}

export function buildStrategyDiscoveryCallbackPayload(job, payload) {
  return {
    job_id: job.job_id,
    completed_at_ns: nowNs(),
    objective: job.objective,
    watchlist: job.watchlist,
    payload,
  };
}

async function loadDfinityModules() {
  const [{ Actor, HttpAgent }, { IDL }, { Ed25519KeyIdentity }, { Principal }] =
    await Promise.all([
      import("@dfinity/agent"),
      import("@dfinity/candid"),
      import("@dfinity/identity"),
      import("@dfinity/principal"),
    ]);
  return { Actor, HttpAgent, IDL, Ed25519KeyIdentity, Principal };
}

function decodeHex(hex) {
  const value = hex.trim().replace(/^0x/, "");
  if (!/^[0-9a-f]+$/i.test(value) || value.length % 2 !== 0) {
    throw new Error("CALLBACK_IDENTITY_SEED_HEX must be valid hex");
  }
  const bytes = new Uint8Array(value.length / 2);
  for (let i = 0; i < value.length; i += 2) {
    bytes[i / 2] = parseInt(value.slice(i, i + 2), 16);
  }
  return bytes;
}

async function callbackActor(env, canisterIdText) {
  const { Actor, HttpAgent, IDL, Ed25519KeyIdentity, Principal } =
    await loadDfinityModules();
  Principal.fromText(canisterIdText);
  const seedHex = String(env.CALLBACK_IDENTITY_SEED_HEX || "").trim();
  if (!seedHex) {
    throw new Error("CALLBACK_IDENTITY_SEED_HEX is not configured");
  }
  const identity = Ed25519KeyIdentity.generate(decodeHex(seedHex));
  const agent = new HttpAgent({
    host: String(env.IC_HOST || DEFAULT_IC_HOST).trim(),
    identity,
    verifyQuerySignatures: false,
  });
  const idlFactory = ({ IDL }) => {
    const ProtocolWatchlistEntry = IDL.Record({
      id: IDL.Text,
      chain_id: IDL.Nat64,
      pool_address: IDL.Text,
      market_data_api_url: IDL.Text,
      abi_api_url: IDL.Text,
    });
    const AbiSelectorAssertion = IDL.Record({
      signature: IDL.Text,
      selector_hex: IDL.Text,
    });
    const ProtocolArtifactBundle = IDL.Record({
      bundle_id: IDL.Text,
      protocol_id: IDL.Text,
      chain_id: IDL.Nat64,
      role: IDL.Text,
      contract_address: IDL.Text,
      abi_json: IDL.Text,
      source_ref: IDL.Text,
      codehash: IDL.Opt(IDL.Text),
      selector_assertions: IDL.Vec(AbiSelectorAssertion),
      spec_summary: IDL.Text,
      risk_notes: IDL.Vec(IDL.Text),
    });
    const ProtocolMarketSnapshot = IDL.Record({
      protocol_id: IDL.Text,
      chain_id: IDL.Nat64,
      pool_address: IDL.Text,
      tvl_usd: IDL.Opt(IDL.Text),
      supply_apy_bps: IDL.Opt(IDL.Nat64),
      borrow_apy_bps: IDL.Opt(IDL.Nat64),
      utilization_bps: IDL.Opt(IDL.Nat64),
      summary: IDL.Text,
      warnings: IDL.Vec(IDL.Text),
    });
    const MarketSynthesisBundle = IDL.Record({
      chain_id: IDL.Nat64,
      generated_at_ns: IDL.Nat64,
      protocols: IDL.Vec(ProtocolMarketSnapshot),
      warnings: IDL.Vec(IDL.Text),
    });
    const StrategyCandidateBundle = IDL.Record({
      candidate_id: IDL.Text,
      objective: IDL.Text,
      protocol_id: IDL.Text,
      primitive: IDL.Text,
      chain_id: IDL.Nat64,
      rationale: IDL.Text,
      required_artifacts: IDL.Vec(IDL.Text),
      assumptions: IDL.Vec(IDL.Text),
      missing_inputs: IDL.Vec(IDL.Text),
      confidence_label: IDL.Text,
      freshness_deadline_ns: IDL.Opt(IDL.Nat64),
      suggested_template_shape: IDL.Opt(IDL.Text),
      estimated_yield_bps: IDL.Opt(IDL.Nat64),
      warnings: IDL.Vec(IDL.Text),
    });
    const StrategyDiscoverySourceType = IDL.Variant({
      OfficialDocs: IDL.Null,
      BlockExplorer: IDL.Null,
      ProtocolApi: IDL.Null,
      MarketDataApi: IDL.Null,
      Other: IDL.Null,
    });
    const StrategyDiscoverySourceTrustTier = IDL.Variant({
      Official: IDL.Null,
      Secondary: IDL.Null,
      BestEffort: IDL.Null,
    });
    const SourceRecord = IDL.Record({
      source_id: IDL.Text,
      source_type: StrategyDiscoverySourceType,
      url: IDL.Text,
      fetched_at_ns: IDL.Nat64,
      content_hash: IDL.Text,
      trust_tier: StrategyDiscoverySourceTrustTier,
    });
    const StrategyDiscoveryResultPayload = IDL.Record({
      protocol_artifacts: IDL.Vec(ProtocolArtifactBundle),
      market: MarketSynthesisBundle,
      candidates: IDL.Vec(StrategyCandidateBundle),
      source_records: IDL.Vec(SourceRecord),
    });
    const SubmitStrategyDiscoveryResultArgs = IDL.Record({
      job_id: IDL.Text,
      completed_at_ns: IDL.Nat64,
      objective: IDL.Text,
      watchlist: IDL.Vec(ProtocolWatchlistEntry),
      payload: StrategyDiscoveryResultPayload,
    });
    const Result = IDL.Variant({ Ok: IDL.Text, Err: IDL.Text });
    return IDL.Service({
      submit_strategy_discovery_result: IDL.Func(
        [SubmitStrategyDiscoveryResultArgs],
        [Result],
        [],
      ),
    });
  };
  return Actor.createActor(idlFactory, {
    agent,
    canisterId: canisterIdText,
  });
}

async function submitCallback(env, job, payload) {
  const actor = await callbackActor(env, job.canister_id);
  const result = await actor[CALLBACK_METHOD](payload);
  if ("Err" in result) {
    throw new Error(`canister callback rejected: ${result.Err}`);
  }
}

async function processJob(env, job) {
  const abiDelayMs = Number(env.ABI_FETCH_DELAY_MS || DEFAULT_ABI_FETCH_DELAY_MS);
  const sourceRecords = [];
  const protocolArtifacts = [];
  const protocols = [];
  const candidates = [];

  for (const entry of job.watchlist) {
    const marketJson = await fetchJson(entry.market_data_api_url);
    sourceRecords.push(buildSourceRecord(`${entry.id}:market`, { MarketDataApi: null }, entry.market_data_api_url));
    const marketSnapshot = buildProtocolMarketSnapshot(entry, marketJson);
    protocols.push(marketSnapshot);

    await delay(abiDelayMs);
    const abiJsonResponse = await fetchJson(entry.abi_api_url);
    sourceRecords.push(buildSourceRecord(`${entry.id}:abi`, { ProtocolApi: null }, entry.abi_api_url));
    const abiJson = normalizeAbiJson(abiJsonResponse);
    protocolArtifacts.push(buildProtocolArtifactBundle(entry, abiJson));
    candidates.push(buildStrategyCandidate(entry, marketSnapshot, job.objective));
  }

  const payload = {
    protocol_artifacts: protocolArtifacts,
    market: {
      chain_id: protocols[0]?.chain_id || 0,
      generated_at_ns: nowNs(),
      protocols,
      warnings: [],
    },
    candidates: candidates.sort((left, right) => {
      const leftYield = left.estimated_yield_bps?.[0] || 0;
      const rightYield = right.estimated_yield_bps?.[0] || 0;
      return rightYield - leftYield;
    }),
    source_records: sourceRecords,
  };
  await submitCallback(env, job, buildStrategyDiscoveryCallbackPayload(job, payload));
}

export async function handleSubmitRequest(request, env, ctx) {
  requireBearerAuth(request, env);
  const text = await request.text();
  if (new TextEncoder().encode(text).length > MAX_REQUEST_BYTES) {
    return json(413, { error: "request too large" });
  }
  let parsed;
  try {
    parsed = parseStrategyDiscoveryJob(JSON.parse(text), env);
  } catch (error) {
    return json(400, { error: toErrorMessage(error) });
  }
  if (!env.STRATEGY_DISCOVERY_QUEUE || typeof env.STRATEGY_DISCOVERY_QUEUE.send !== "function") {
    return json(500, { error: "STRATEGY_DISCOVERY_QUEUE binding is not configured" });
  }
  await env.STRATEGY_DISCOVERY_QUEUE.send(parsed);
  return json(202, {
    job_id: parsed.job_id,
    accepted_at_ns: nowAckNs(),
    status: "accepted",
  });
}

export default {
  async fetch(request, env, ctx) {
    const url = new URL(request.url);
    if (request.method === "GET" && url.pathname === HEALTH_ROUTE) {
      return json(200, { ok: true });
    }
    if (request.method === "POST" && url.pathname === SUBMIT_ROUTE) {
      try {
        return await handleSubmitRequest(request, env, ctx);
      } catch (error) {
        return json(401, { error: toErrorMessage(error) });
      }
    }
    return json(404, { error: "not found" });
  },

  async queue(batch, env, ctx) {
    for (const message of batch.messages) {
      try {
        await processJob(env, message.body);
        message.ack();
      } catch (error) {
        console.error(`strategy discovery job failed: ${toErrorMessage(error)}`);
        message.retry();
      }
    }
  },
};
