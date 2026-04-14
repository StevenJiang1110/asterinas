#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/run_kata_alpine.sh

Installs the local Kata test environment and runs one or more full Alpine
smoke-test passes.

Environment:
  KATA_PASSES              Number of passes to run. Default: 1.
  KATA_CGROUP_NAMESPACE    Cgroup namespace for `nerdctl run`. Default: private.
  CRICTL_VERSION           Optional `crictl` version. Default: v1.29.0.
  KATA_VERSION             Kata release version. Default: 3.28.0.
  NERDCTL_VERSION          `nerdctl` release version. Default: v2.2.2.
  PAUSE_IMAGE              Pause image for inner `containerd`. Default:
                           registry.k8s.io/pause:3.10.
EOF
}

kata_handle_help_or_no_args show_help "$@"

: "${CRICTL_VERSION:=v1.29.0}"
: "${KATA_VERSION:=3.28.0}"
: "${NERDCTL_VERSION:=v2.2.2}"
: "${PAUSE_IMAGE:=registry.k8s.io/pause:3.10}"
: "${KATA_CGROUP_NAMESPACE:=private}"
export CRICTL_VERSION KATA_CGROUP_NAMESPACE KATA_VERSION NERDCTL_VERSION PAUSE_IMAGE

pass_count="${KATA_PASSES:-1}"

check_local_cgroup_delegation() {
  if [ ! -f /sys/fs/cgroup/cgroup.controllers ]; then
    return 0
  fi

  probe_dir="/sys/fs/cgroup/kata-make-preflight-$$"
  mkdir "${probe_dir}"
  if ! printf '+cpu\n' > "${probe_dir}/cgroup.subtree_control" 2>/dev/null; then
    rmdir "${probe_dir}" 2>/dev/null || true
    echo "Local cgroup v2 hierarchy cannot delegate controllers required by Kata." >&2
    echo "Run this inside a dev container started with a delegable cgroup setup (the CI job uses --privileged --cgroupns host)." >&2
    exit 1
  fi
  rmdir "${probe_dir}" 2>/dev/null || true
}

if ! [[ "${pass_count}" =~ ^[1-9][0-9]*$ ]]; then
  echo "KATA_PASSES must be a positive integer, got: ${pass_count}" >&2
  exit 1
fi

# Local runs still need the same outer cgroup delegation that CI uses.
check_local_cgroup_delegation

# Install the shared Kata test toolchain once before running any pass.
bash "${script_dir}/install_kata_env.sh"

for pass_index in $(seq 1 "${pass_count}"); do
  echo "==> Kata local run pass ${pass_index}/${pass_count}"
  bash "${script_dir}/run_kata_pass.sh"
done
