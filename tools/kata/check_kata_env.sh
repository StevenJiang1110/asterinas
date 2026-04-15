#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/check_kata_env.sh

Verifies that the background Kata and `containerd` services are ready before
running the smoke workload.

Environment:
  KATA_CONFIG_FILE  Optional Bash config fragment. Default:
                    tools/kata/config/smoke-test.env.
  KATA_CHECK_DEBUG  Set to 1/true/yes to print `kata-runtime check -v` output
                    during successful runs too.
EOF
}

kata_handle_help_or_no_args show_help "$@"
kata_load_config "${script_dir}/config/smoke-test.env"

should_print_kata_check_output() {
  case "${KATA_CHECK_DEBUG:-0}" in
    1 | true | TRUE | yes | YES)
      return 0
      ;;
  esac

  return 1
}

emit_github_error() {
  title="$1"
  file_path="$2"
  if [ ! -f "${file_path}" ]; then
    return 0
  fi

  message="$(python3 -c 'import pathlib, sys; text = pathlib.Path(sys.argv[1]).read_text(); text = text.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A"); print(text[:6000])' "${file_path}")"
  echo "::error title=${title}::${message}"
}

print_grouped_file() {
  group_name="$1"
  file_path="$2"

  echo "::group::${group_name}"
  cat "${file_path}" 2>/dev/null || true
  echo "::endgroup::"
}

summarize_kata_check_strace() {
  grep -nE '/dev/kvm|KVM_CREATE_VM|EINVAL|EPERM|ENODEV|EBUSY' /tmp/kata-check.strace > /tmp/kata-check.strace.summary || true
}

report_kvm_probe_failure() {
  summarize_kata_check_strace
  emit_github_error "KVM create probe" /tmp/kvm-create-vm.txt
  emit_github_error "kata-runtime check" /tmp/kata-check.txt
  emit_github_error "kata-runtime strace" /tmp/kata-check.strace.summary
  print_grouped_file "kvm-create-vm.txt" /tmp/kvm-create-vm.txt
  print_grouped_file "kata-check.txt" /tmp/kata-check.txt
  print_grouped_file "kata-check.strace" /tmp/kata-check.strace.summary
  print_grouped_file "containerd.log" /tmp/containerd.log
  print_grouped_file "kata-syslog.log" /tmp/kata-syslog.log
}

wait_for_containerd_ready() {
  timeout 60 bash -c '
    until [ -S "${CONTAINERD_ADDRESS}" ] &&
      ctr --address "${CONTAINERD_ADDRESS}" plugins ls >/tmp/ctr-plugins.txt 2>/dev/null &&
      awk '\''$1 == "io.containerd.grpc.v1" && $2 == "cri" && $NF == "ok" { found = 1 } END { exit(found ? 0 : 1) }'\'' /tmp/ctr-plugins.txt; do
      sleep 1
    done
  '
}

# Wait for the background services started by `start_kata_services.sh`.
wait_for_containerd_ready

# Print a short environment summary before running deeper checks.
uname -a
sed -n "1,8p" /etc/os-release
ls -l /dev/kvm /dev/vhost-vsock || true
kata-runtime --show-default-config-paths

containerd --version
runc --version
nerdctl --version
kata-runtime --version

test -S "${CONTAINERD_ADDRESS}"
ctr --address "${CONTAINERD_ADDRESS}" plugins ls > /tmp/ctr-plugins.txt
awk '$1 == "io.containerd.grpc.v1" && $2 == "cri" && $NF == "ok" { found = 1 } END { exit(found ? 0 : 1) }' /tmp/ctr-plugins.txt
if command -v crictl >/dev/null; then
  crictl --version
  crictl --runtime-endpoint "unix://${CONTAINERD_ADDRESS}" --image-endpoint "unix://${CONTAINERD_ADDRESS}" info > /tmp/crictl-info.json
  jq -e 'has("config") and has("status")' /tmp/crictl-info.json >/dev/null
else
  echo "crictl not installed; skipping CRI info probe."
fi

grep -F 'runtime_type = "io.containerd.kata.v2"' /etc/containerd/config.toml
grep -F 'ConfigPath = "/etc/kata-containers/configuration.toml"' /etc/containerd/config.toml

nerdctl --address "${CONTAINERD_ADDRESS}" info > /tmp/nerdctl-info.txt
kata-runtime env > /tmp/kata-env.txt
grep -E '/etc/kata-containers/configuration.toml|/opt/kata/share/defaults/kata-containers/' /tmp/kata-env.txt

# Keep the direct KVM probe as the hard gate and `kata-runtime check` as a
# diagnostic that still prints useful logs on partial failures.
kvm_probe_status=0
python3 tools/kata/check_kvm_create_vm.py 2>&1 | tee /tmp/kvm-create-vm.txt || kvm_probe_status=$?
kata_check_status=0
if should_print_kata_check_output; then
  strace -f -o /tmp/kata-check.strace -s 256 kata-runtime check -v 2>&1 | tee /tmp/kata-check.txt || kata_check_status=$?
else
  strace -f -o /tmp/kata-check.strace -s 256 kata-runtime check -v >/tmp/kata-check.txt 2>&1 || kata_check_status=$?
fi

if [ "${kvm_probe_status}" -ne 0 ]; then
  report_kvm_probe_failure
  echo "KVM create probe failed; skipping nerdctl workload attempt."
  exit 1
fi

if [ "${kata_check_status}" -ne 0 ]; then
  summarize_kata_check_strace
  echo "::warning title=kata-runtime check::kata-runtime check failed, but the direct KVM probe succeeded; continuing with the configured nerdctl workload."
  emit_github_error "kata-runtime check" /tmp/kata-check.txt
  emit_github_error "kata-runtime strace" /tmp/kata-check.strace.summary
fi
