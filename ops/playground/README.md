# Playground VPS Runtime

This directory is the checked-in runtime contract for the shared playground VPS:

- `docker-compose.yml` manages `anvil`, `web`, `indexer`, and `rpc-gateway`
- `Caddyfile` is the host-managed TLS/router config
- `systemd/icp-playground.service` keeps the local ICP runtime outside Docker
- `playground.env.example` is the shared env contract for Compose, Caddy, `systemd`, and the bootstrap/reset scripts

For the first-time machine setup, use [VPS-SETUP.md](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/VPS-SETUP.md). This README stays focused on the runtime contract and the steady-state deploy model.

If the VPS is on your Tailnet, the setup guide now treats Tailscale as the preferred admin path for SSH and deploy traffic, while keeping the user-facing web and RPC hostnames public.

## Host layout

Use one operator-owned env file, for example `/etc/automaton-playground/playground.env`, plus one state tree such as `/srv/automaton-playground/`.

The example env keeps these host-written files under `PLAYGROUND_STATE_DIR` so the host scripts and the containerized indexer read the same paths:

- `playground-status.json`
- `factory-canister-id.txt`
- `local-escrow-deployment.json`
- `indexer.sqlite`

## Install

1. Copy and edit the shared env file.

```sh
sudo install -d /etc/automaton-playground /srv/automaton-playground/state /srv/automaton-playground/services /srv/automaton-playground/artifacts
sudo cp ops/playground/playground.env.example /etc/automaton-playground/playground.env
sudo ${EDITOR:-vi} /etc/automaton-playground/playground.env
```

2. Install the ICP `systemd` unit and start it.

```sh
sudo cp ops/playground/systemd/icp-playground.service /etc/systemd/system/icp-playground.service
sudo systemctl daemon-reload
sudo systemctl enable --now icp-playground
```

3. Make the same env file available to Caddy, then install the checked-in Caddy config.

```sh
sudo systemctl edit caddy
```

Add:

```ini
[Service]
EnvironmentFile=/etc/automaton-playground/playground.env
```

Then:

```sh
sudo cp ops/playground/Caddyfile /etc/caddy/Caddyfile
sudo systemctl restart caddy
```

4. Validate the Compose file and start the core runtime services.

```sh
docker compose --env-file /etc/automaton-playground/playground.env -f ops/playground/docker-compose.yml config
docker compose --env-file /etc/automaton-playground/playground.env -f ops/playground/docker-compose.yml up -d
```

## Bootstrap and Reset

Load the same env file before running the repo-owned bootstrap/reset scripts:

```sh
set -a
. /etc/automaton-playground/playground.env
set +a
```

With the example env, the VPS mode is:

- `PLAYGROUND_MANAGE_SERVICES=0`
- `PLAYGROUND_ANVIL_MANAGED=0`

That means:

- Compose owns `anvil`, `web`, `indexer`, and `rpc-gateway`
- `icp-playground.service` owns the local ICP runtime
- `scripts/playground-bootstrap.sh` deploys the factory/escrow stack, writes `factory-canister-id.txt`, waits for the loopback services to become healthy, and runs smoke checks

Run:

```sh
sh ./scripts/playground-bootstrap.sh
```

Hard reset:

```sh
sh ./scripts/playground-reset.sh
```

## Optional Profiles

Otterscan is intentionally loopback-only in this layout because raw Anvil stays private. If you enable it, use SSH tunnels for operator access.

```sh
docker compose --env-file /etc/automaton-playground/playground.env -f ops/playground/docker-compose.yml --profile otterscan up -d otterscan
```

Grafana is also profile-gated and loopback-only:

```sh
docker compose --env-file /etc/automaton-playground/playground.env -f ops/playground/docker-compose.yml --profile monitoring up -d grafana
```

## Release creation and actions

The reusable `Publish Atomic Playground Release` workflow checks out one clean
commit, runs the full Plan 004 contract gates, builds the factory and child
Wasm artifacts, publishes digest-addressed images, and uploads one immutable
bundle containing the schema-v2 manifest, exact bytes, checksums, and a
source/ops archive. It does not deploy an environment.

The manifest is shaped like [`ops/playground/release-manifest.example.json`](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/release-manifest.example.json).
It contains source provenance, image digests, raw Wasm digests, and the ops
revision. It contains no deployment mode or secret.

The four actions are intentionally separate:

1. `--mode soft` updates only the `web`, `indexer`, and `rpc-gateway` images,
   then runs smoke checks. It never uploads an artifact, reinstalls the
   factory, resets Anvil, or upgrades a child.
2. `--mode hard-reset` is the manual environment-approved reset path. It uses
   the manifest-selected factory and child bytes while recreating ephemeral
   playground state.
3. `--mode admit-child` uploads the selected child bytes to the existing
   factory and verifies factory health. It does not upgrade existing children.
4. `--mode upgrade-named --canister-id <principal>` takes a pre-upgrade
   snapshot and upgrades exactly that canister with ICP upgrade mode. The VPS
   requires `PLAYGROUND_UPGRADE_APPROVED=1` in addition to the protected CI
   environment.

The mode is selected by the workflow or command line, never read from the
immutable manifest. Manual local validation can render a manifest without
deploying:

```sh
npm run build:factory-wasm
./components/ic-automaton/scripts/build-backend-wasm.sh dist/automaton.wasm
node scripts/render-release-manifest.mjs --mode dry-run --output tmp/release-manifest.json
```

Manual example:

```sh
set -a
. /etc/automaton-playground/playground.env
set +a

bash ./scripts/deploy-playground-release.sh --manifest /path/to/release-manifest.json
```

If `GHCR_USERNAME` and `GHCR_TOKEN` are exported, the soft/hard deploy script
logs into `ghcr.io` before pulling the exact image digests from the manifest.

The script records each applied manifest under `PLAYGROUND_RELEASES_DIR` and keeps `current.json` there as the latest deployed manifest.

If you want to publish a release without touching the VPS, use [`Publish Atomic Playground Release`](../../.github/workflows/publish-playground-images.yml). Its single bundle is the source of truth for all three image refs and both Wasm artifacts.

Rollback is component-scoped: select a prior manifest and repeat only the
requested action. Rolling back images does not admit a factory artifact or
upgrade existing children; rolling back an admitted artifact does not change
running containers or existing children.

## Notes

- The Compose file is written for a Linux VPS. `indexer` uses host networking so it can reach the host-managed local ICP replica on `127.0.0.1`.
- The containerized indexer waits for `PLAYGROUND_FACTORY_CANISTER_ID_FILE` by default. That file is written by `scripts/playground-bootstrap.sh` after the factory canister is deployed or reinstalled.
- `PLAYGROUND_WEB_IMAGE`, `PLAYGROUND_INDEXER_IMAGE`, and `PLAYGROUND_RPC_GATEWAY_IMAGE` should point at CI-built images. Do not rebuild ad hoc on the VPS once Phase 10 release automation lands.
