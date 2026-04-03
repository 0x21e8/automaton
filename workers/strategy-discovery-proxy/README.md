# Strategy Discovery Proxy Worker

This worker accepts bounded strategy-discovery jobs from the canister, runs a deterministic fetch-and-normalize pipeline off-chain, and calls back into `submit_strategy_discovery_result`.

## Responsibilities

- Accept `POST /v1/strategy-discovery/jobs` submit payloads.
- Require bearer auth from the canister submitter.
- Enforce bounded request size and curated source-host allowlists.
- Queue accepted jobs for asynchronous processing.
- Fetch market JSON and ABI JSON for each configured watchlist entry.
- Build typed protocol-artifact, market-synthesis, and candidate bundles.
- Call back into the canister using a stable worker identity.

## Required Environment

- `STRATEGY_DISCOVERY_WORKER_API_KEY`
- `CALLBACK_IDENTITY_SEED_HEX`
- `IC_HOST`
- `CURATED_SOURCE_HOSTS`

Optional:

- `BASESCAN_API_KEY`
- `ABI_FETCH_DELAY_MS`

## Local Check

```bash
node --test workers/strategy-discovery-proxy/test/worker.test.mjs
```
