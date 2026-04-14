#!/usr/bin/env bash

set -euo pipefail

script_dir="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=tools/kata/common.sh
source "${script_dir}/common.sh"

show_help() {
  cat <<'EOF'
Usage: bash tools/kata/install_kata_env.sh

Installs the distro packages and userspace binaries required by the Kata
smoke-test helpers.

Environment:
  CRICTL_VERSION       Optional `crictl` version. Default: v1.29.0.
  KATA_VERSION         Kata release version. Default: 3.28.0.
  NERDCTL_VERSION      `nerdctl` release version. Default: v2.2.2.
  KATA_INSTALL_CRICTL  Set to 1/true/yes to install `crictl`.
  KATA_FORCE_APT       Set to 1/true/yes to force `apt-get update && apt-get
                       install` even when the required distro packages are
                       already present.
  KATA_PAYLOAD_IMAGE   Kata payload image. Default:
                       quay.io/kata-containers/kata-deploy:${KATA_VERSION}.
EOF
}

kata_handle_help_or_no_args show_help "$@"

download_release_asset() {
  output_path="$1"
  download_url="$2"

  if [ -f "${output_path}" ]; then
    curl --continue-at - --fail --location --retry 5 --retry-all-errors --silent --show-error \
      --output "${output_path}" \
      "${download_url}"
    return
  fi

  curl --fail --location --retry 5 --retry-all-errors --silent --show-error \
    --output "${output_path}" \
    "${download_url}"
}

wait_for_socket() {
  socket_path="$1"

  for _ in $(seq 1 30); do
    if [ -S "${socket_path}" ]; then
      return 0
    fi
    sleep 1
  done

  echo "Timed out waiting for socket: ${socket_path}" >&2
  return 1
}

need_nerdctl_install() {
  ! command -v nerdctl >/dev/null ||
    ! nerdctl --version 2>/dev/null | grep -F " ${NERDCTL_VERSION#v}" >/dev/null
}

need_crictl_install() {
  ! command -v crictl >/dev/null ||
    ! crictl --version 2>/dev/null | grep -F "${CRICTL_VERSION}" >/dev/null
}

should_install_crictl() {
  case "${KATA_INSTALL_CRICTL:-0}" in
    1 | true | TRUE | yes | YES)
      return 0
      ;;
  esac

  return 1
}

need_kata_install() {
  [ ! -x /opt/kata/bin/kata-runtime ] ||
    [ ! -x /opt/kata/bin/containerd-shim-kata-v2 ] ||
    ! /opt/kata/bin/kata-runtime --version 2>/dev/null | grep -F "${KATA_VERSION}" >/dev/null
}

should_force_apt_install() {
  case "${KATA_FORCE_APT:-0}" in
    1 | true | TRUE | yes | YES)
      return 0
      ;;
  esac

  return 1
}

install_required_packages() {
  packages=(
    busybox-syslogd
    containernetworking-plugins
    containerd
    iptables
    jq
    kmod
    python3
    runc
    strace
    zstd
  )
  missing_packages=()

  if ! should_force_apt_install; then
    for package_name in "${packages[@]}"; do
      if ! dpkg-query -W -f='${Status}\n' "${package_name}" 2>/dev/null | grep -Fqx 'install ok installed'; then
        missing_packages+=("${package_name}")
      fi
    done
  fi

  if should_force_apt_install || [ "${#missing_packages[@]}" -gt 0 ]; then
    apt-get update
    if should_force_apt_install; then
      apt-get install -y "${packages[@]}"
    else
      apt-get install -y "${missing_packages[@]}"
    fi
  fi
}

install_kata_from_payload_image() {
  installer_root="${KATA_INSTALLER_CONTAINERD_ROOT:-/var/lib/kata-installer-containerd}"
  installer_state="${KATA_INSTALLER_CONTAINERD_STATE:-/run/kata-installer-containerd.$$}"
  installer_socket="${KATA_INSTALLER_CONTAINERD_ADDRESS:-${installer_state}/containerd.sock}"
  installer_log="${KATA_INSTALLER_CONTAINERD_LOG:-/tmp/kata-installer-containerd.log}"
  payload_pull_log="${KATA_INSTALLER_PULL_LOG:-/tmp/kata-installer-pull.log}"
  installer_extract_dir="${KATA_INSTALLER_EXTRACT_DIR:-/tmp/kata-installer-extract}"
  payload_image="${KATA_PAYLOAD_IMAGE:-quay.io/kata-containers/kata-deploy:${KATA_VERSION}}"
  installer_pid=

  cleanup_installer() {
    rm -rf "${installer_extract_dir}"

    if [ -n "${installer_pid}" ]; then
      kill "${installer_pid}" 2>/dev/null || true
      wait_for_exit "${installer_pid}"
    fi

    rm -rf "${installer_state}"
  }

  trap cleanup_installer RETURN

  rm -f "${installer_socket}"
  install -d -m 0755 "$(dirname "${installer_socket}")" "${installer_root}" "${installer_state}"
  rm -f "${installer_log}" "${payload_pull_log}"

  # A temporary `containerd` instance lets us unpack the official Kata payload
  # image without pulling extra repo-specific tooling into the helper.
  containerd \
    --address "${installer_socket}" \
    --root "${installer_root}" \
    --state "${installer_state}" \
    >"${installer_log}" 2>&1 &
  installer_pid=$!
  wait_for_socket "${installer_socket}"

  if ! ctr \
    --address "${installer_socket}" \
    images pull \
    --local \
    --snapshotter native \
    --platform linux/amd64 \
    "${payload_image}" \
    >"${payload_pull_log}" 2>&1; then
    tail -n 200 "${payload_pull_log}" >&2 || true
    return 1
  fi

  image_index_digest="$(
    ctr --address "${installer_socket}" images inspect --content "${payload_image}" |
      awk '/application\/vnd\.docker\.distribution\.manifest.list\.v2\+json @sha256:/ { sub(/^.*@/, "", $0); sub(/ .*/, "", $0); print; exit }'
  )"
  amd64_manifest_digest="$(
    ctr --address "${installer_socket}" content get "${image_index_digest}" |
      python3 -c 'import json, sys; obj = json.load(sys.stdin); print(next(manifest["digest"] for manifest in obj["manifests"] if manifest["platform"]["os"] == "linux" and manifest["platform"]["architecture"] == "amd64"))'
  )"
  layer_digests="$(
    ctr --address "${installer_socket}" content get "${amd64_manifest_digest}" |
      python3 -c 'import json, sys; obj = json.load(sys.stdin); print("\n".join(layer["digest"] for layer in sorted(obj["layers"], key=lambda layer: layer["size"], reverse=True)))'
  )"

  layer_blob_path=
  while read -r layer_digest; do
    [ -z "${layer_digest}" ] && continue

    candidate_blob_path="${installer_root}/io.containerd.content.v1.content/blobs/sha256/${layer_digest#sha256:}"
    if gzip -dc "${candidate_blob_path}" | tar -tf - opt/kata-artifacts/opt/kata/bin/kata-runtime >/dev/null 2>&1; then
      layer_blob_path="${candidate_blob_path}"
      break
    fi
  done <<< "${layer_digests}"

  test -n "${layer_blob_path}"
  rm -rf "${installer_extract_dir}"
  install -d -m 0755 "${installer_extract_dir}"
  gzip -dc "${layer_blob_path}" | tar -xf - -C "${installer_extract_dir}" opt/kata-artifacts/opt/kata

  test -d "${installer_extract_dir}/opt/kata-artifacts/opt/kata"
  rm -rf /opt/kata
  cp -a "${installer_extract_dir}/opt/kata-artifacts/opt/kata" /opt/
  test -x /opt/kata/bin/kata-runtime
  test -x /opt/kata/bin/containerd-shim-kata-v2
}

# Install the shared package dependencies used by local and workflow Kata runs.
install_required_packages

if need_nerdctl_install; then
  download_release_asset \
    /tmp/nerdctl.tgz \
    "https://github.com/containerd/nerdctl/releases/download/${NERDCTL_VERSION}/nerdctl-${NERDCTL_VERSION#v}-linux-amd64.tar.gz"
  tar -C /usr/local/bin -xzf /tmp/nerdctl.tgz nerdctl
fi

if should_install_crictl && need_crictl_install; then
  download_release_asset \
    /tmp/crictl.tgz \
    "https://github.com/kubernetes-sigs/cri-tools/releases/download/${CRICTL_VERSION}/crictl-${CRICTL_VERSION}-linux-amd64.tar.gz"
  tar -C /usr/local/bin -xzf /tmp/crictl.tgz crictl
fi

if need_kata_install; then
  install_kata_from_payload_image
fi

# Expose the installed Kata binaries on the default `PATH`.
install -d -m 0755 /usr/local/bin
ln -sf /opt/kata/bin/kata-runtime /usr/local/bin/kata-runtime
ln -sf /opt/kata/bin/containerd-shim-kata-v2 /usr/local/bin/containerd-shim-kata-v2
