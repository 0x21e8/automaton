import test from "node:test";
import assert from "node:assert/strict";

import {
  buildStrategyDiscoveryCallbackPayload,
  handleSubmitRequest,
  parseStrategyDiscoveryJob,
} from "../src/worker.js";

function sampleJobPayload() {
  return {
    canister_id: "aaaaa-aa",
    job_id: "sd-job-1",
    objective: "find reserve opportunities",
    watchlist: [
      {
        id: "moonwell-usdc",
        chain_id: 8453,
        pool_address: "0x1111111111111111111111111111111111111111",
        market_data_api_url: "https://api.example.com/market/moonwell",
        abi_api_url: "https://api.example.com/abi/moonwell",
      },
    ],
    exposure_summary: "active_exposures=0",
    autonomy_summary: "preserve runway",
  };
}

function workerEnv() {
  const queued = [];
  return {
    queued,
    env: {
      STRATEGY_DISCOVERY_WORKER_API_KEY: "secret",
      CURATED_SOURCE_HOSTS: "api.example.com",
      STRATEGY_DISCOVERY_QUEUE: {
        async send(message) {
          queued.push(message);
        },
      },
    },
  };
}

test("parseStrategyDiscoveryJob validates and normalizes watchlist input", () => {
  const { env } = workerEnv();
  const parsed = parseStrategyDiscoveryJob(sampleJobPayload(), env);
  assert.equal(parsed.job_id, "sd-job-1");
  assert.equal(parsed.watchlist[0].pool_address, "0x1111111111111111111111111111111111111111");
});

test("handleSubmitRequest enqueues accepted discovery jobs", async () => {
  const { env, queued } = workerEnv();
  const request = new Request("https://worker.example/v1/strategy-discovery/jobs", {
    method: "POST",
    headers: {
      authorization: "Bearer secret",
      "content-type": "application/json",
    },
    body: JSON.stringify(sampleJobPayload()),
  });
  const response = await handleSubmitRequest(request, env, {});
  assert.equal(response.status, 202);
  const body = await response.json();
  assert.equal(body.job_id, "sd-job-1");
  assert.equal(body.status, "accepted");
  assert.equal(queued.length, 1);
  assert.equal(queued[0].objective, "find reserve opportunities");
});

test("handleSubmitRequest rejects requests outside the curated host allowlist", async () => {
  const { env } = workerEnv();
  const body = sampleJobPayload();
  body.watchlist[0].market_data_api_url = "https://evil.example.net/market/moonwell";
  const request = new Request("https://worker.example/v1/strategy-discovery/jobs", {
    method: "POST",
    headers: {
      authorization: "Bearer secret",
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
  const response = await handleSubmitRequest(request, env, {});
  assert.equal(response.status, 400);
  const payload = await response.json();
  assert.match(payload.error, /not in curated allowlist/);
});

test("buildStrategyDiscoveryCallbackPayload matches the canister callback contract", () => {
  const job = parseStrategyDiscoveryJob(sampleJobPayload(), workerEnv().env);
  const payload = buildStrategyDiscoveryCallbackPayload(job, {
    protocol_artifacts: [],
    market: {
      chain_id: 8453,
      generated_at_ns: 1n,
      protocols: [],
      warnings: [],
    },
    candidates: [],
    source_records: [],
  });
  assert.equal(payload.job_id, "sd-job-1");
  assert.equal(payload.objective, "find reserve opportunities");
  assert.equal(payload.watchlist.length, 1);
  assert.equal(payload.payload.market.chain_id, 8453);
});
