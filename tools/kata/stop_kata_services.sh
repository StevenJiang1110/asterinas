#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/stop_kata_services.sh

Stops the background services started by the Kata smoke-test helpers.
EOF
}

kata_handle_help_or_no_args show_help "$@"

if [ -f /tmp/containerd.pid ]; then
  containerd_pid="$(cat /tmp/containerd.pid)"
  kill "${containerd_pid}" 2>/dev/null || true
  wait_for_exit "${containerd_pid}"
  rm -f /tmp/containerd.pid
fi

if [ -f /tmp/syslogd.pid ]; then
  syslogd_pid="$(cat /tmp/syslogd.pid)"
  kill "${syslogd_pid}" 2>/dev/null || true
  wait_for_exit "${syslogd_pid}"
  rm -f /tmp/syslogd.pid
fi
