#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/test_nerdctl_alpine.sh

Runs the real `nerdctl` + Kata Alpine smoke test against the already-started
inner `containerd` daemon.

Environment:
  CONTAINERD_ADDRESS         Inner `containerd` socket. Default:
                             /run/containerd/containerd.sock.
  KATA_ALPINE_IMAGE          Smoke-test image. Default:
                             quay.io/libpod/alpine:latest.
  KATA_NERDCTL_DEBUG         Set to 1/true/yes to add `--debug-full` and print
                             `nerdctl run` output during successful runs too.
  KATA_NERDCTL_RUNTIME       Runtime name. Default: io.containerd.kata.v2.
  KATA_CGROUP_NAMESPACE      `nerdctl run --cgroupns` value. Default: host.
  KATA_CGROUP_PARENT         Host cgroup parent. Default: /kata-ci.
  KATA_ALPINE_RELEASE_REGEX  Expected Alpine release regex.
EOF
}

kata_handle_help_or_no_args show_help "$@"

should_enable_nerdctl_debug() {
  case "${KATA_NERDCTL_DEBUG:-0}" in
    1 | true | TRUE | yes | YES)
      return 0
      ;;
  esac

  return 1
}

emit_github_error_tail() {
  title="$1"
  file_path="$2"
  if [ ! -f "${file_path}" ]; then
    return 0
  fi

  message="$(python3 -c 'import pathlib, sys; text = pathlib.Path(sys.argv[1]).read_text(); text = text[-6000:]; text = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A"); print(text)' "${file_path}")"
  echo "::error title=${title}::${message}"
}

containerd_address="${CONTAINERD_ADDRESS:-/run/containerd/containerd.sock}"
image="${KATA_ALPINE_IMAGE:-quay.io/libpod/alpine:latest}"
runtime="${KATA_NERDCTL_RUNTIME:-io.containerd.kata.v2}"
expected_release_regex="${KATA_ALPINE_RELEASE_REGEX:-^[0-9]+\\.[0-9]+(\\.|$)}"
cgroup_namespace="${KATA_CGROUP_NAMESPACE:-host}"
cgroup_parent="${KATA_CGROUP_PARENT:-/kata-ci}"
cgroup_parent_path="/sys/fs/cgroup${cgroup_parent}"
pull_log_file="${KATA_PULL_LOG_FILE:-/tmp/nerdctl-pull.txt}"
run_log_file="${KATA_RUN_LOG_FILE:-/tmp/nerdctl-run-command.txt}"
test_log_file="${KATA_TEST_LOG_FILE:-/tmp/nerdctl-run.txt}"

ensure_cgroup_parent() {
  if [ ! -f /sys/fs/cgroup/cgroup.controllers ]; then
    return 0
  fi

  mkdir -p "${cgroup_parent_path}"
  available_controllers="$(cat "${cgroup_parent_path}/cgroup.controllers")"
  enabled_controllers="$(cat "${cgroup_parent_path}/cgroup.subtree_control")"
  controllers_to_enable=()

  for controller in cpu cpuset; do
    if grep -qw "${controller}" <<< "${available_controllers}" &&
      ! grep -qw "${controller}" <<< "${enabled_controllers}"; then
      controllers_to_enable+=("+${controller}")
    fi
  done

  if [ "${#controllers_to_enable[@]}" -gt 0 ]; then
    printf '%s\n' "${controllers_to_enable[*]}" > "${cgroup_parent_path}/cgroup.subtree_control"
  fi
}

cleanup() {
  rc=$?

  if [ "${rc}" -ne 0 ]; then
    if [ -s "${run_log_file}" ]; then
      emit_github_error_tail "nerdctl run" "${run_log_file}"
      echo "::group::nerdctl-run-command.txt"
      cat "${run_log_file}" || true
      echo "::endgroup::"
    fi
    if [ -s "${pull_log_file}" ]; then
      emit_github_error_tail "nerdctl pull" "${pull_log_file}"
      echo "::group::nerdctl-pull.txt"
      cat "${pull_log_file}" || true
      echo "::endgroup::"
    fi
    if [ -s "${test_log_file}" ]; then
      emit_github_error_tail "nerdctl alpine" "${test_log_file}"
      echo "::group::nerdctl-run.txt"
      cat "${test_log_file}" || true
      echo "::endgroup::"
    fi
    if [ -f /tmp/containerd.log ]; then
      emit_github_error_tail "containerd log" /tmp/containerd.log
      echo "::group::containerd.log"
      cat /tmp/containerd.log || true
      echo "::endgroup::"
    fi
    if [ -f /tmp/kata-syslog.log ]; then
      emit_github_error_tail "kata syslog" /tmp/kata-syslog.log
      echo "::group::kata-syslog.log"
      cat /tmp/kata-syslog.log || true
      echo "::endgroup::"
    fi
  fi

  rm -f "${release_output_file:-}" "${test_log_file}"
}

: > "${pull_log_file}"
: > "${run_log_file}"
: > "${test_log_file}"

release_output_file="$(mktemp)"
trap cleanup EXIT

cgroup_parent_args=()
nerdctl_debug_args=()
if [ "${cgroup_namespace}" = "host" ]; then
  # When sharing the host cgroup namespace, pre-create the delegateable parent
  # that Kata's shim expects to place the VM workload under.
  ensure_cgroup_parent
  cgroup_parent_args=(--cgroup-parent "${cgroup_parent}")
fi

if should_enable_nerdctl_debug; then
  nerdctl_debug_args=(--debug-full)
fi

nerdctl --snapshotter native --address "${containerd_address}" pull --quiet "${image}" \
  2>&1 | tee "${pull_log_file}"

nerdctl "${nerdctl_debug_args[@]}" --address "${containerd_address}" run \
  --rm \
  --cgroup-manager cgroupfs \
  "${cgroup_parent_args[@]}" \
  --cgroupns "${cgroup_namespace}" \
  --net none \
  --snapshotter native \
  --runtime "${runtime}" \
  "${image}" \
  cat /etc/alpine-release >"${run_log_file}" 2>&1

cp "${run_log_file}" "${test_log_file}"
cp "${run_log_file}" "${release_output_file}"

grep -E "${expected_release_regex}" "${release_output_file}"
ls -l /dev/kvm || true
