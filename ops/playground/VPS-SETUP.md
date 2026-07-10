# Playground VPS Setup

This guide is the first-time setup path for a fresh Linux VPS that will run the shared Automaton playground.

It assumes:

- one Linux VPS
- host-managed `caddy`
- host-managed `systemd`
- host-managed local ICP runtime via `icp`
- Compose-managed `anvil`, `web`, `indexer`, and `rpc-gateway`
- Tailscale is available for operator access and deploy traffic

It does not assume ad hoc builds on the VPS after setup. Ongoing deploys should use release manifests and [`scripts/deploy-playground-release.sh`](/Users/domwoe/Dev/projects/automaton-launchpad/scripts/deploy-playground-release.sh).

## 1. Install host dependencies

Install these on the VPS:

- `docker` and the Docker Compose plugin
- `caddy`
- `git`
- Node.js 24.x and npm 11.x
- Rust/Cargo
- `forge`, `cast`, and `anvil`
- `icp`
- `tailscale`

The repo scripts use all of them directly:

- `icp build` and `icp network start` for the local IC runtime
- `forge` and `cast` for escrow deploy/smoke/faucet flows
- `docker compose` for the long-lived runtime services

Tailscale is not required for the public user-facing path, but it is the preferred admin path in this setup:

- operator SSH over Tailnet
- optional GitHub Actions deploy traffic over Tailnet
- operator dashboards and diagnostics kept private on Tailnet or loopback

## 2. Prepare the filesystem layout

Use one repo checkout plus one operator-owned state tree.

Recommended paths:

- repo: `/srv/automaton-launchpad/current`
- env file: `/etc/automaton-playground/playground.env`
- state: `/srv/automaton-playground/`

Create the directories:

```sh
sudo install -d \
  /etc/automaton-playground \
  /srv/automaton-playground/state \
  /srv/automaton-playground/services \
  /srv/automaton-playground/artifacts \
  /srv/automaton-playground/icp-home \
  /srv/automaton-playground/icp-state
```

Clone the repo to the path used by the checked-in `systemd` unit:

```sh
sudo install -d /srv/automaton-launchpad
sudo chown -R "$USER" /srv/automaton-launchpad /srv/automaton-playground
git clone <repo-url> /srv/automaton-launchpad/current
cd /srv/automaton-launchpad/current
```

## 3. Create the shared env file

Copy [playground.env.example](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/playground.env.example) into place:

```sh
cp ops/playground/playground.env.example /etc/automaton-playground/playground.env
```

Set at least these values before continuing:

- `PLAYGROUND_DOMAIN`
- `PLAYGROUND_RPC_DOMAIN`
- `PLAYGROUND_ACME_EMAIL`
- `PLAYGROUND_PUBLIC_RPC_URL`
- `INDEXER_CORS_ALLOWED_ORIGINS`
- `LOCAL_EVM_FORK_URL`
- `PLAYGROUND_VPS_REPO_ROOT` is not in the env file; if you use a different repo path, update the `systemd` unit and GitHub Actions variable instead
- `PLAYGROUND_ENV_VERSION` for legacy/manual bootstrap only; release workflows
  derive provenance from the manifest

For manual bring-up before a VPS deploy, point the runtime images at immutable digest refs that were published by GitHub Actions:

- `PLAYGROUND_WEB_IMAGE`
- `PLAYGROUND_INDEXER_IMAGE`
- `PLAYGROUND_RPC_GATEWAY_IMAGE`

Use the `Publish Atomic Playground Release` workflow in GitHub Actions. It
publishes the three image digests and both canister Wasms from one commit, then
uploads a single release bundle. Deploy workflows stage the source/ops archive
from that bundle and verify its revision before running any action.

Keep these Phase 5/10 defaults unchanged unless you have a reason to change the topology:

- `PLAYGROUND_MANAGE_SERVICES=0`
- `PLAYGROUND_ANVIL_MANAGED=0`
- `PLAYGROUND_REQUIRE_FORK=1`
- `PLAYGROUND_CHAIN_ID=20260326`

## 4. Install the local ICP systemd unit

Install [systemd/icp-playground.service](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/systemd/icp-playground.service):

```sh
sudo cp ops/playground/systemd/icp-playground.service /etc/systemd/system/icp-playground.service
sudo systemctl daemon-reload
sudo systemctl enable --now icp-playground
```

Check it:

```sh
systemctl status icp-playground --no-pager
```

This service is responsible only for the local ICP runtime. It does not manage the Docker services.

## 5. Install the Caddy config

Make Caddy read the same env file used by Compose and the repo scripts:

```sh
sudo systemctl edit caddy
```

Add:

```ini
[Service]
EnvironmentFile=/etc/automaton-playground/playground.env
```

Then install [Caddyfile](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/Caddyfile):

```sh
sudo cp ops/playground/Caddyfile /etc/caddy/Caddyfile
sudo systemctl restart caddy
```

Check it:

```sh
systemctl status caddy --no-pager
```

The routing contract is:

- `https://$PLAYGROUND_DOMAIN` -> web app plus indexer routes
- `https://$PLAYGROUND_RPC_DOMAIN` -> RPC gateway

Raw Anvil and the local ICP replica stay private on loopback.

## 6. Join the VPS to the Tailnet

Install and authenticate Tailscale on the VPS:

```sh
curl -fsSL https://tailscale.com/install.sh | sh
sudo tailscale up
```

Record the VPS Tailnet identity you want to use for SSH and CI deploys:

- the Tailscale IPv4 address, or
- the MagicDNS hostname

If this VPS is already in the same Tailnet, verify it:

```sh
tailscale status
tailscale ip -4
```

Recommended access model:

- public internet: only `80/tcp` and `443/tcp`
- Tailnet only: SSH, Otterscan, Grafana, and any ad hoc operator access

Example `ufw` rules if you want SSH reachable only via `tailscale0`:

```sh
sudo ufw default deny incoming
sudo ufw allow 80/tcp
sudo ufw allow 443/tcp
sudo ufw allow in on tailscale0 to any port 22 proto tcp
sudo ufw enable
```

## 7. Validate and start the Compose services

Validate the checked-in Compose graph:

```sh
docker compose \
  --env-file /etc/automaton-playground/playground.env \
  -f ops/playground/docker-compose.yml \
  config
```

Start the core services:

```sh
docker compose \
  --env-file /etc/automaton-playground/playground.env \
  -f ops/playground/docker-compose.yml \
  up -d
```

This starts:

- `anvil`
- `web`
- `indexer`
- `rpc-gateway`

At this point the indexer may still be waiting on `PLAYGROUND_FACTORY_CANISTER_ID_FILE`. That is expected before bootstrap.

## 8. Bootstrap the playground

Load the shared env file into the shell:

```sh
set -a
. /etc/automaton-playground/playground.env
set +a
```

Run the repo-owned bootstrap:

```sh
sh ./scripts/playground-bootstrap.sh
```

This is the first real environment initialization. It will:

- mark maintenance on in the playground status file
- ensure the local ICP network is running
- ensure Anvil is reachable on the configured fork and chain ID
- deploy the escrow contracts
- deploy or reinstall the factory
- upload the child Wasm artifact
- write `PLAYGROUND_FACTORY_CANISTER_ID_FILE`
- wait for the loopback indexer and RPC gateway
- run `scripts/playground-smoke.sh`
- mark maintenance off only after smoke succeeds

After bootstrap, normal operations use the explicit release workflows:

- `Deploy Soft Playground Release`
- `Deploy Hard Reset Playground Release`
- `Admit Child Artifact`
- `Upgrade Named Automaton`

The first action is the normal public-service rollout. The other three are
protected/manual actions with separate blast radii. Existing child automatons
are never implicitly upgraded by a release or image rollback.

## 9. Validate over Tailscale first, then validate the public surface

Before you trust DNS/TLS or open public traffic, validate the loopback services from the VPS and, if convenient, from another Tailnet machine.

On the VPS:

```sh
curl -fsS http://127.0.0.1:3001/health | jq
curl -fsS http://127.0.0.1:3001/api/playground | jq
curl -fsS http://127.0.0.1:3002 \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' | jq
```

If you expose SSH only on Tailscale, this is also a good point to verify you can reach the VPS over its Tailscale IP or MagicDNS hostname from your operator machine.

Then validate the public endpoints:

After bootstrap, check the public endpoints:

```sh
curl -fsS "https://$PLAYGROUND_DOMAIN/health" | jq
curl -fsS "https://$PLAYGROUND_DOMAIN/api/playground" | jq
curl -fsS "https://$PLAYGROUND_RPC_DOMAIN" \
  -H 'content-type: application/json' \
  -d '{"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}' | jq
```

Expected results:

- the indexer `/health` route responds
- `/api/playground` returns the playground metadata
- `eth_chainId` returns `0x13525e6`

Also run the smoke script once directly:

```sh
sh ./scripts/playground-smoke.sh
```

## 10. Configure GitHub Actions deploy access

The checked-in workflows in [.github/workflows/deploy-soft.yml](/Users/domwoe/Dev/projects/automaton-launchpad/.github/workflows/deploy-soft.yml) and [.github/workflows/deploy-hard-reset.yml](/Users/domwoe/Dev/projects/automaton-launchpad/.github/workflows/deploy-hard-reset.yml) expect a GitHub environment named `playground-vps`.

Add these environment secrets:

- `PLAYGROUND_VPS_HOST`
- `PLAYGROUND_VPS_PORT`
- `PLAYGROUND_VPS_USER`
- `PLAYGROUND_VPS_KNOWN_HOSTS`
- `PLAYGROUND_VPS_SSH_KEY`
- `PLAYGROUND_GHCR_USERNAME`
- `PLAYGROUND_GHCR_TOKEN`
- `PLAYGROUND_CHILD_WASM_URL`
- `PLAYGROUND_CHILD_WASM_SHA256`
- `PLAYGROUND_CHILD_VERSION_COMMIT`
- `PLAYGROUND_TAILSCALE_AUTHKEY` if the workflow should join the Tailnet before SSH

Add these environment variables:

- `PLAYGROUND_VPS_REPO_ROOT`
- `PLAYGROUND_FORK_BLOCK_NUMBER` if you want a pinned fork block

Before the first deploy, you can also run the `Publish Playground Images` workflow to prime GHCR and download the image refs artifact. Use the digest-pinned values from that artifact to populate `PLAYGROUND_WEB_IMAGE`, `PLAYGROUND_INDEXER_IMAGE`, and `PLAYGROUND_RPC_GATEWAY_IMAGE` in your VPS env file.

The deploy workflows SSH to the VPS, upload a release manifest, and run:

```sh
bash ./scripts/deploy-playground-release.sh --manifest /tmp/<manifest>.json
```

Tailnet mode:

- set `PLAYGROUND_VPS_HOST` to the VPS Tailscale IP or MagicDNS hostname
- set `PLAYGROUND_TAILSCALE_AUTHKEY` so the GitHub-hosted runner can join the Tailnet before `scp` and `ssh`

Non-Tailnet mode:

- leave `PLAYGROUND_TAILSCALE_AUTHKEY` unset
- point `PLAYGROUND_VPS_HOST` at the normal SSH endpoint

No workflow is supposed to `git pull` or rebuild on the VPS.

## 11. Manual deploy and hard reset commands

Manual soft deploy with an already prepared manifest:

```sh
set -a
. /etc/automaton-playground/playground.env
set +a

export GHCR_USERNAME='<ghcr-user>'
export GHCR_TOKEN='<ghcr-token>'

bash ./scripts/deploy-playground-release.sh --manifest /path/to/release-manifest.json
```

Manual hard reset deploy:

```sh
bash ./scripts/deploy-playground-release.sh --manifest /path/to/release-manifest.json --mode hard-reset
```

The deploy script will:

- validate the manifest shape
- pull exact image digests
- fetch or verify the child Wasm artifact
- update the runtime services
- run either a soft deploy or a full `playground-reset.sh`
- record the applied manifest under `PLAYGROUND_RELEASES_DIR`

## 12. Troubleshooting

If the indexer never comes up after bootstrap:

- check `PLAYGROUND_FACTORY_CANISTER_ID_FILE`
- check `docker compose ... logs indexer`
- check that `icp-playground.service` is healthy and listening on the configured local replica port

If the hard reset path fails to refresh Anvil:

- confirm `PLAYGROUND_ANVIL_MANAGED=0`
- confirm the deploy script is invoking `playground-reset.sh` rather than running a hand-written Docker command
- confirm `docker compose -f ops/playground/docker-compose.yml up -d --force-recreate --no-deps anvil` works on the host

If Caddy serves the site but wallets cannot connect:

- verify `PLAYGROUND_PUBLIC_RPC_URL` matches the public RPC hostname
- verify `https://$PLAYGROUND_RPC_DOMAIN` returns `eth_chainId`
- verify raw Anvil is still loopback-only and the gateway is the only public JSON-RPC surface

If GitHub Actions cannot reach the VPS in Tailnet mode:

- check that `PLAYGROUND_TAILSCALE_AUTHKEY` is set in the `playground-vps` environment
- check that `PLAYGROUND_VPS_HOST` is the VPS Tailscale IP or MagicDNS hostname, not its public IP
- check `tailscale status` output in the workflow logs before the SSH step
- confirm the VPS is already online in the same Tailnet and allows SSH via Tailscale

## Related Files

- high-level runtime contract: [README.md](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/README.md)
- env reference: [playground.env.example](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/playground.env.example)
- Compose services: [docker-compose.yml](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/docker-compose.yml)
- Caddy routing: [Caddyfile](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/Caddyfile)
- local ICP `systemd` unit: [systemd/icp-playground.service](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/systemd/icp-playground.service)
- release manifest shape: [release-manifest.example.json](/Users/domwoe/Dev/projects/automaton-launchpad/ops/playground/release-manifest.example.json)
- deploy entrypoint: [deploy-playground-release.sh](/Users/domwoe/Dev/projects/automaton-launchpad/scripts/deploy-playground-release.sh)
