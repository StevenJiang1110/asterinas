#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/run_kata_smoke.sh

Installs the local Kata test environment and runs one or more configurable
smoke-test passes.

Environment:
  KATA_CONFIG_FILE  Optional Bash config fragment. Default:
                    tools/kata/config/smoke-test.env.
  KATA_PASSES       Number of passes to run.
EOF
}

kata_handle_help_or_no_args show_help "$@"
kata_load_config "${script_dir}/config/smoke-test.env"

pass_count="${KATA_PASSES}"

check_local_cgroup_delegation() {
  if [ ! -f /sys/fs/cgroup/cgroup.controllers ]; then
    return 0
  fi

  probe_dir="/sys/fs/cgroup/kata-script-preflight-$$"
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
  echo "==> Kata smoke-test pass ${pass_index}/${pass_count}"
  bash "${script_dir}/run_kata_pass.sh"
done
