#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/run_kata_pass.sh

Runs one full Kata smoke-test pass:
  1. start background services
  2. validate the environment
  3. run the Alpine workload
  4. stop background services
EOF
}

kata_handle_help_or_no_args show_help "$@"

cleanup() {
  bash "${script_dir}/stop_kata_services.sh"
}

trap cleanup EXIT

bash "${script_dir}/start_kata_services.sh"
bash "${script_dir}/check_kata_env.sh"
bash "${script_dir}/test_nerdctl_alpine.sh"
