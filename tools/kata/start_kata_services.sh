#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
config_dir="${script_dir}/config"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/start_kata_services.sh

Installs the repo-owned Kata and `containerd` configs, then starts the
background services used by the smoke test.

Environment:
  KATA_CONFIG_FILE  Optional Bash config fragment. Default:
                    tools/kata/config/smoke-test.env.
  PAUSE_IMAGE       Pause image for inner `containerd`.
EOF
}

kata_handle_help_or_no_args show_help "$@"
kata_load_config "${script_dir}/config/smoke-test.env"

select_kata_config_source() {
  if [ -f /opt/kata/share/defaults/kata-containers/configuration.toml ]; then
    echo /opt/kata/share/defaults/kata-containers/configuration.toml
    return
  fi

  echo /opt/kata/share/defaults/kata-containers/configuration-qemu.toml
}

select_qemu_binary_path() {
  qemu_candidates=(
    /opt/kata/bin/qemu-system-x86_64
    /usr/local/qemu/bin/qemu-system-x86_64
    /usr/bin/qemu-system-x86_64
    /usr/bin/qemu-kvm
    /usr/libexec/qemu-kvm
    /usr/lib/qemu/qemu-system-x86_64
  )

  if command -v qemu-system-x86_64 >/dev/null 2>&1; then
    command -v qemu-system-x86_64
    return 0
  fi

  for qemu_candidate in "${qemu_candidates[@]}"; do
    if [ -x "${qemu_candidate}" ]; then
      printf '%s\n' "${qemu_candidate}"
      return 0
    fi
  done

  return 1
}

normalize_qemu_config_path() {
  kata_config_path="$1"

  if ! qemu_binary_path="$(select_qemu_binary_path)"; then
    echo "Cannot find a usable QEMU binary for Kata." >&2
    return 1
  fi

  sed -i \
    -e 's#^\(path = \)".*"#\1"'"${qemu_binary_path}"'"#' \
    -e 's#^\(valid_hypervisor_paths = \)\[.*\]#\1["'"${qemu_binary_path}"'"]#' \
    "${kata_config_path}"
}

install_repo_configs() {
  : "${PAUSE_IMAGE:?PAUSE_IMAGE must be set}"

  kata_config_source="$(select_kata_config_source)"
  test -f "${kata_config_source}"

  # Start from Kata's upstream base config, then layer the repo-owned drop-in.
  install -d -m 0755 /etc/kata-containers /etc/kata-containers/config.d
  install -m 0644 "${kata_config_source}" /etc/kata-containers/configuration.toml
  normalize_qemu_config_path /etc/kata-containers/configuration.toml
  install -m 0644 "${config_dir}/kata-10-container.toml" /etc/kata-containers/config.d/10-container.toml

  # Materialize the CNI and `containerd` configs that the nested stack expects.
  install -d -m 0755 /opt/cni /etc/cni/net.d /etc/containerd /run/containerd /var/lib/containerd
  if [ ! -e /opt/cni/bin ]; then
    ln -s /usr/lib/cni /opt/cni/bin
  fi
  install -m 0644 "${config_dir}/cni-10-kata.conflist" /etc/cni/net.d/10-kata.conflist
  sed "s|__PAUSE_IMAGE__|${PAUSE_IMAGE}|g" "${config_dir}/containerd-config.toml.in" > /etc/containerd/config.toml
}

prepare_host_prerequisites() {
  modprobe overlay || true
  modprobe br_netfilter || true
  sysctl -w net.ipv4.ip_forward=1 || true
  sysctl -w net.bridge.bridge-nf-call-iptables=1 || true
  iptables -P FORWARD ACCEPT
}

start_background_services() {
  # Reset any previous helper state before starting a fresh pass.
  bash "${script_dir}/stop_kata_services.sh"
  rm -f /dev/log /tmp/containerd.log /tmp/kata-syslog.log

  nohup syslogd -n -O /tmp/kata-syslog.log >/tmp/kata-syslog.stdout 2>&1 &
  echo $! > /tmp/syslogd.pid

  timeout 10 bash -c '
    until [ -S /dev/log ]; do
      sleep 1
    done
  '

  nohup containerd --config /etc/containerd/config.toml --log-level debug >/tmp/containerd.log 2>&1 &
  echo $! > /tmp/containerd.pid
}

install_repo_configs
prepare_host_prerequisites
start_background_services
