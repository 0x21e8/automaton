# 1) Kill any launcher/replica that may be running as another user
  pkill -f pocket-ic || true
  pkill -f icp-cli-network-launcher || true

  # 2) Verify with elevated visibility (important)
  sudo lsof -nP -iTCP:8000 -sTCP:LISTEN || true

  # 3) Remove stale local lock/state markers
  rm -rf .icp/cache/networks/local
  rm -rf /tmp/icp-home-local
  rm -f .icp/identity/.lock

  # 4) Start local network again
  icp network start -d