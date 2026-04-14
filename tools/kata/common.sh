#!/usr/bin/env bash

# Shared helpers for the Kata setup and smoke-test scripts.

kata_handle_help_or_no_args() {
  local usage_fn="$1"
  shift

  case "${1:-}" in
    -h | --help)
      "${usage_fn}"
      exit 0
      ;;
  esac

  if [ "$#" -ne 0 ]; then
    echo "Unexpected arguments: $*" >&2
    echo >&2
    "${usage_fn}" >&2
    exit 1
  fi
}

wait_for_exit() {
  local process_id="$1"

  for _ in $(seq 1 50); do
    if ! kill -0 "${process_id}" 2>/dev/null; then
      return 0
    fi
    sleep 0.2
  done

  kill -9 "${process_id}" 2>/dev/null || true
}
