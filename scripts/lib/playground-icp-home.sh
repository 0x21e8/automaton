#!/bin/sh

initialize_playground_icp_home() {
  mkdir -p "$PLAYGROUND_ICP_HOME/port-descriptors"
}

ensure_playground_icp_network() {
  project_root=$1
  network_name=$2
  ping_attempts=${3:-60}

  if icp --project-root-override "$project_root" network ping "$network_name" >/dev/null 2>&1; then
    return 0
  fi

  # A failed ping may clean a stale descriptor, including its parent directory.
  # Recreate the isolated descriptor path immediately before starting the network.
  initialize_playground_icp_home
  icp --project-root-override "$project_root" network start --background "$network_name"

  attempt=0
  while [ "$attempt" -lt "$ping_attempts" ]; do
    if icp --project-root-override "$project_root" network ping "$network_name" >/dev/null 2>&1; then
      return 0
    fi

    attempt=$((attempt + 1))
    sleep 1
  done

  echo "PocketIC network $network_name did not become ready on port" >&2
  return 1
}
