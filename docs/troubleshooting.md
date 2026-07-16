# Troubleshooting

## Local ICP says it is running, but the replica is actually dead

How it manifested:

- `icp network start --background` returned `Error: network 'local' is already running`
- `icp network ping local` failed with errors like:
  - `Error: no descriptor found for port 8000`
  - `Error: An error happened during communication with the replica: error sending request for url (http://localhost:8000/api/v2/status)`
- nothing useful was listening on `127.0.0.1:8000`, or only a stale launcher process remained after `pocket-ic` had already died

What happened:

- the local `icp-cli-network-launcher` / `pocket-ic` process crashed or was interrupted
- stale state was left behind in this repo's `.icp/cache/networks/local` and in the shared `ICP_HOME` port descriptors
- after that, `icp` believed the project-local network still existed even though the replica was gone

Quick resolve:

```bash
# 1. Stop any stale launcher / pocket-ic processes if they still exist
pkill -f icp-cli-network-launcher || true
pkill -f pocket-ic || true

# 2. Clear the broken local-network metadata
rm -rf .icp/cache/networks/local
rm -f "${ICP_HOME:-$HOME/.icp}/port-descriptors/8000.json"
rm -f "${ICP_HOME:-$HOME/.icp}/port-descriptors/8000.lock"

# 3. Start the local network again and verify it
ICP_HOME=/tmp/icp-home icp network start --background
ICP_HOME=/tmp/icp-home icp network ping local
```

If you are using a non-default `ICP_HOME`, keep it consistent for every `icp` command in the session. Mixed homes can make the network look missing or make canister IDs disappear even though the replica is healthy.
