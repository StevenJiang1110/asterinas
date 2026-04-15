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
  KATA_CONFIG_FILE    Optional Bash config fragment. Default:
                      tools/kata/config/smoke-test.env.
  CRICTL_VERSION       Optional `crictl` version. Default: v1.29.0.
  KATA_VERSION         Kata release version. Default: 3.28.0.
  NERDCTL_VERSION      `nerdctl` release version. Default: v2.2.2.
  KATA_INSTALL_CRICTL  Set to 1/true/yes to install `crictl`.
  KATA_FORCE_APT       Set to 1/true/yes to force `apt-get update && apt-get
                       install` even when the required distro packages are
                       already present.
  KATA_PAYLOAD_IMAGE   Kata payload image. Default:
                       quay.io/kata-containers/kata-deploy:${KATA_VERSION}.
  KATA_ASTERINAS_KERNEL_PATH
                       Optional local `aster-kernel-osdk-bin.qemu_elf` path.
                       When present, patches the official Kata base install the
                       same way as the `jjf-dev/kata-containers` Asterinas
                       release workflow.
  KATA_STATIC_TARBALL_URL
                       Optional Kata static tarball URL. When set, installs
                       Kata directly from that tarball instead of the payload
                       image flow.
  KATA_STATIC_TARBALL_SHA256_URL
                       Optional checksum URL for the static tarball. Default:
                       derived from `KATA_STATIC_TARBALL_URL` by replacing
                       `.tar.zst` with `.SHA256SUMS`.
  KATA_STATIC_TARBALL_SHA256
                       Optional expected SHA256 for the static tarball. When
                       set, skips remote checksum lookup.
  KATA_STATIC_TARBALL_CACHE_DIR
                       Cache directory for downloaded static tarballs. Default:
                       `/var/cache/kata-static`.
EOF
}

kata_handle_help_or_no_args show_help "$@"
kata_load_config "${script_dir}/config/smoke-test.env"

download_release_asset() {
  output_path="$1"
  download_url="$2"

  if command -v wget >/dev/null 2>&1; then
    if [ -f "${output_path}" ]; then
      wget --continue --tries=5 --output-document "${output_path}" "${download_url}"
      return
    fi

    wget --tries=5 --output-document "${output_path}" "${download_url}"
    return
  fi

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

release_asset_basename() {
  asset_url="$1"
  basename "${asset_url%%\?*}"
}

derive_static_tarball_sha256_url() {
  static_tarball_url="${KATA_STATIC_TARBALL_URL:?KATA_STATIC_TARBALL_URL must be set}"

  if [ -n "${KATA_STATIC_TARBALL_SHA256_URL:-}" ]; then
    printf '%s\n' "${KATA_STATIC_TARBALL_SHA256_URL}"
    return
  fi

  if [[ "${static_tarball_url}" == *.tar.zst ]]; then
    printf '%s\n' "${static_tarball_url%.tar.zst}.SHA256SUMS"
    return
  fi

  echo "Cannot derive SHA256SUMS URL from static tarball URL: ${static_tarball_url}" >&2
  return 1
}

resolve_static_tarball_sha256() {
  static_tarball_url="${KATA_STATIC_TARBALL_URL:?KATA_STATIC_TARBALL_URL must be set}"
  static_tarball_name="$(release_asset_basename "${static_tarball_url}")"

  if [ -n "${KATA_STATIC_TARBALL_SHA256:-}" ]; then
    printf '%s\n' "${KATA_STATIC_TARBALL_SHA256}"
    return
  fi

  if [ -n "${KATA_STATIC_TARBALL_RESOLVED_SHA256:-}" ]; then
    printf '%s\n' "${KATA_STATIC_TARBALL_RESOLVED_SHA256}"
    return
  fi

  checksum_url="$(derive_static_tarball_sha256_url)"
  checksum_download_path="${KATA_STATIC_SHA256_DOWNLOAD_PATH:-/tmp/kata-static-sha256.$$}"
  download_release_asset "${checksum_download_path}" "${checksum_url}"

  resolved_sha256="$(
    awk -v asset_name="${static_tarball_name}" '
      $2 == asset_name || $2 ~ ("/" asset_name "$") {
        print $1
        exit
      }
    ' "${checksum_download_path}"
  )"

  if [ -z "${resolved_sha256}" ]; then
    echo "Cannot resolve SHA256 for static tarball: ${static_tarball_name}" >&2
    return 1
  fi

  KATA_STATIC_TARBALL_RESOLVED_SHA256="${resolved_sha256}"
  export KATA_STATIC_TARBALL_RESOLVED_SHA256
  printf '%s\n' "${resolved_sha256}"
}

prepare_cached_static_tarball() {
  static_tarball_url="${KATA_STATIC_TARBALL_URL:?KATA_STATIC_TARBALL_URL must be set}"
  cache_dir="${KATA_STATIC_TARBALL_CACHE_DIR:-/var/cache/kata-static}"
  static_tarball_name="$(release_asset_basename "${static_tarball_url}")"
  cached_tarball_path="${cache_dir}/${static_tarball_name}"
  cached_tarball_hash_path="${cached_tarball_path}.sha256"
  expected_sha256="$(resolve_static_tarball_sha256)"

  install -d -m 0755 "${cache_dir}"

  if [ -f "${cached_tarball_path}" ]; then
    cached_sha256="$(sha256sum "${cached_tarball_path}" | awk '{print $1}')"
    if [ "${cached_sha256}" = "${expected_sha256}" ]; then
      printf '%s\n' "${expected_sha256}" > "${cached_tarball_hash_path}"
      printf '%s\n' "${cached_tarball_path}"
      return
    fi
  fi

  temp_tarball_path="${cached_tarball_path}.download.$$"
  rm -f "${temp_tarball_path}"
  download_release_asset "${temp_tarball_path}" "${static_tarball_url}"

  downloaded_sha256="$(sha256sum "${temp_tarball_path}" | awk '{print $1}')"
  if [ "${downloaded_sha256}" != "${expected_sha256}" ]; then
    echo "Downloaded static tarball SHA256 mismatch: expected ${expected_sha256}, got ${downloaded_sha256}" >&2
    rm -f "${temp_tarball_path}"
    return 1
  fi

  mv "${temp_tarball_path}" "${cached_tarball_path}"
  printf '%s\n' "${expected_sha256}" > "${cached_tarball_hash_path}"
  printf '%s\n' "${cached_tarball_path}"
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
  source_marker_path="${KATA_INSTALL_SOURCE_MARKER:-/opt/kata/.kata-install-source}"
  asterinas_kernel_path="${KATA_ASTERINAS_KERNEL_PATH:-}"

  if [ ! -x /opt/kata/bin/kata-runtime ] ||
    [ ! -x /opt/kata/bin/containerd-shim-kata-v2 ] ||
    ! /opt/kata/bin/kata-runtime --version 2>/dev/null | grep -F "${KATA_VERSION}" >/dev/null; then
    return 0
  fi

  if [ -n "${asterinas_kernel_path}" ] && [ -f "${asterinas_kernel_path}" ]; then
    [ ! -f "${source_marker_path}" ] ||
      ! grep -Fqx "asterinas-kernel-overlay ${asterinas_kernel_path}" "${source_marker_path}"
    return
  fi

  if [ -f "${source_marker_path}" ] &&
    grep -Fq "asterinas-kernel-overlay " "${source_marker_path}"; then
    return 0
  fi

  if [ -n "${KATA_STATIC_TARBALL_URL:-}" ]; then
    expected_sha256="$(resolve_static_tarball_sha256)"
    installed_static_tarball_url=
    installed_static_tarball_sha256=
    if [ -f "${source_marker_path}" ]; then
      installed_static_tarball_url="$(sed -n 's/^static-tarball-url //p' "${source_marker_path}" | head -n 1)"
      installed_static_tarball_sha256="$(sed -n 's/^static-tarball-sha256 //p' "${source_marker_path}" | head -n 1)"
    fi
    [ ! -f "${source_marker_path}" ] ||
      [ "${installed_static_tarball_url}" != "${KATA_STATIC_TARBALL_URL}" ] ||
      [ "${installed_static_tarball_sha256}" != "${expected_sha256}" ]
    return
  fi

  payload_image="${KATA_PAYLOAD_IMAGE:-quay.io/kata-containers/kata-deploy:${KATA_VERSION}}"
  [ ! -f "${source_marker_path}" ] ||
    ! grep -Fqx "payload-image ${payload_image}" "${source_marker_path}"
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
  source_marker_path="${KATA_INSTALL_SOURCE_MARKER:-/opt/kata/.kata-install-source}"
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
  printf 'payload-image %s\n' "${payload_image}" > "${source_marker_path}"
  test -x /opt/kata/bin/kata-runtime
  test -x /opt/kata/bin/containerd-shim-kata-v2
}

install_kata_from_static_tarball() {
  source_marker_path="${KATA_INSTALL_SOURCE_MARKER:-/opt/kata/.kata-install-source}"
  static_tarball_url="${KATA_STATIC_TARBALL_URL:?KATA_STATIC_TARBALL_URL must be set}"
  extract_dir="${KATA_STATIC_EXTRACT_DIR:-/tmp/kata-static-extract}"
  static_tarball_sha256="$(resolve_static_tarball_sha256)"
  static_tarball_path="$(prepare_cached_static_tarball)"

  rm -rf "${extract_dir}"
  install -d -m 0755 "${extract_dir}"
  tar --zstd -xf "${static_tarball_path}" -C "${extract_dir}"

  test -d "${extract_dir}/opt/kata"
  rm -rf /opt/kata
  cp -a "${extract_dir}/opt/kata" /opt/
  {
    printf 'static-tarball-url %s\n' "${static_tarball_url}"
    printf 'static-tarball-sha256 %s\n' "${static_tarball_sha256}"
  } > "${source_marker_path}"
  test -x /opt/kata/bin/kata-runtime
  test -x /opt/kata/bin/containerd-shim-kata-v2
}

patch_qemu_config_for_asterinas() {
  source_config="$1"
  dest_config="$2"

  cp "${source_config}" "${dest_config}"
  sed -i \
    -e 's#^kernel = ".*"#kernel = "/opt/kata/share/kata-containers/aster-kernel-osdk-bin.qemu_elf"#' \
    -e 's#^image = ".*"#initrd = "/opt/kata/share/kata-containers/kata-containers-initrd.img"#' \
    -e 's#^initrd = ".*"#initrd = "/opt/kata/share/kata-containers/kata-containers-initrd.img"#' \
    "${dest_config}"
}

install_kata_from_asterinas_kernel_overlay() {
  source_marker_path="${KATA_INSTALL_SOURCE_MARKER:-/opt/kata/.kata-install-source}"
  asterinas_kernel_path="${KATA_ASTERINAS_KERNEL_PATH:?KATA_ASTERINAS_KERNEL_PATH must be set}"
  share_dir=/opt/kata/share/kata-containers
  defaults_dir=/opt/kata/share/defaults/kata-containers
  runtime_rs_defaults_dir="${defaults_dir}/runtime-rs"
  payload_image="${KATA_PAYLOAD_IMAGE:-quay.io/kata-containers/kata-deploy:${KATA_VERSION}}"

  if [ ! -x /opt/kata/bin/kata-runtime ] ||
    [ ! -x /opt/kata/bin/containerd-shim-kata-v2 ] ||
    ! /opt/kata/bin/kata-runtime --version 2>/dev/null | grep -F "${KATA_VERSION}" >/dev/null; then
    install_kata_from_payload_image
  fi

  test -f "${asterinas_kernel_path}"
  test -d "${share_dir}"
  test -d "${defaults_dir}"

  install -m 0755 "${asterinas_kernel_path}" "${share_dir}/aster-kernel-osdk-bin.qemu_elf"
  ln -sfn "aster-kernel-osdk-bin.qemu_elf" "${share_dir}/vmlinuz.container"
  ln -sfn "aster-kernel-osdk-bin.qemu_elf" "${share_dir}/vmlinux.container"

  patch_qemu_config_for_asterinas \
    "${defaults_dir}/configuration-qemu.toml" \
    "${defaults_dir}/configuration-asterinas.toml"
  ln -sfn "configuration-asterinas.toml" "${defaults_dir}/configuration.toml"

  if [ -f "${runtime_rs_defaults_dir}/configuration-qemu-runtime-rs.toml" ]; then
    patch_qemu_config_for_asterinas \
      "${runtime_rs_defaults_dir}/configuration-qemu-runtime-rs.toml" \
      "${runtime_rs_defaults_dir}/configuration-asterinas-runtime-rs.toml"
    ln -sfn "configuration-asterinas-runtime-rs.toml" "${runtime_rs_defaults_dir}/configuration.toml"
  fi

  printf 'asterinas-kernel-overlay %s\n' "${asterinas_kernel_path}" > "${source_marker_path}"
  printf 'payload-image %s\n' "${payload_image}" >> "${source_marker_path}"
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
  if [ -n "${KATA_ASTERINAS_KERNEL_PATH:-}" ] && [ -f "${KATA_ASTERINAS_KERNEL_PATH}" ]; then
    install_kata_from_asterinas_kernel_overlay
  elif [ -n "${KATA_STATIC_TARBALL_URL:-}" ]; then
    install_kata_from_static_tarball
  else
    install_kata_from_payload_image
  fi
fi

# Expose the installed Kata binaries on the default `PATH`.
install -d -m 0755 /usr/local/bin
ln -sf /opt/kata/bin/kata-runtime /usr/local/bin/kata-runtime
ln -sf /opt/kata/bin/containerd-shim-kata-v2 /usr/local/bin/containerd-shim-kata-v2
