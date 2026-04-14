#!/bin/sh

# SPDX-License-Identifier: MPL-2.0

set -e

# Default values
CONFIG_PATH=""
DISK=""
FORCE_FORMAT_DISK=false

# Function to display help
show_help() {
    cat << EOF
Usage:
$0 --config <CONFIG_PATH> --disk <DISK> [--force-format-disk]
$0 [-h | --help]

Options:
  --config <CONFIG_PATH>      Path to the configuration file.
  --disk <DISK>               Target disk for installation (e.g., /dev/sda).
  --force-format-disk         Forcefully format the specified disk (DANGEROUS!).
  -h, --help                  Show this help message.

Example:
  $0 --config ./distro/configuration.nix --disk /dev/vda --force-format-disk
EOF
}

# Parse command line arguments
while [[ $# -gt 0 ]]; do
    case "$1" in
        --config)
            if [[ -z "$2" ]] || [[ "$2" == -* ]]; then
                echo "Error: --config requires a non-empty argument." >&2
                exit 1
            fi
            CONFIG_PATH="$2"
            shift 2
            ;;
        --disk)
            if [[ -z "$2" ]] || [[ "$2" == -* ]]; then
                echo "Error: --disk requires a non-empty argument." >&2
                exit 1
            fi
            DISK="$2"
            shift 2
            ;;
        --force-format-disk)
            FORCE_FORMAT_DISK=true
            shift
            ;;
        -h|--help)
            show_help
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            show_help
            exit 1
            ;;
    esac
done

# Validate required arguments
if [[ -z "$CONFIG_PATH" ]]; then
    echo "Error: --config is required." >&2
    exit 1
fi

if [[ -z "$DISK" ]]; then
    echo "Error: --disk is required." >&2
    exit 1
fi

# Confirm dangerous operation if --force-format-disk is used
if [[ "$FORCE_FORMAT_DISK" == true ]]; then
    echo "WARNING: You are about to FORMAT the disk: $DISK"
    sgdisk --zap-all $DISK
    partprobe $DISK
fi

BUILD_DIR=$(mktemp -d -p /mnt)

partition_sysfs_path() {
    printf '/sys/class/block/%s/dev' "$(basename "$1")"
}

partition_exists() {
    [ -b "$1" ] || [ -e "$(partition_sysfs_path "$1")" ]
}

ensure_partition_device_node() {
    partition_path="$1"
    sysfs_dev_path=$(partition_sysfs_path "$partition_path")

    if [ -b "$partition_path" ]; then
        return 0
    fi

    if [ ! -e "$sysfs_dev_path" ]; then
        return 1
    fi

    device_numbers=$(cat "$sysfs_dev_path")
    major_number=${device_numbers%%:*}
    minor_number=${device_numbers##*:}
    mknod "$partition_path" b "$major_number" "$minor_number"
}

refresh_partition_table() {
    partprobe "$DISK" >/dev/null 2>&1 || true

    if command -v partx >/dev/null 2>&1; then
        partx -u "$DISK" >/dev/null 2>&1 || true
    fi
}

ensure_partition_devices() {
    for _ in $(seq 1 10); do
        refresh_partition_table

        boot_ready=false
        root_ready=false

        if ensure_partition_device_node "$BOOT_DEVICE"; then
            boot_ready=true
        fi
        if ensure_partition_device_node "$ROOT_DEVICE"; then
            root_ready=true
        fi

        if [ "$boot_ready" = true ] && [ "$root_ready" = true ]; then
            return 0
        fi

        sleep 1
    done

    echo "Error: cannot access partition devices ${BOOT_DEVICE} and ${ROOT_DEVICE}." >&2
    return 1
}

if [ "${DISK#/dev/loop}" != "$DISK" ]; then
    BOOT_DEVICE="${DISK}p1"
    ROOT_DEVICE="${DISK}p2"
else
    BOOT_DEVICE="${DISK}1"
    ROOT_DEVICE="${DISK}2"
fi
if ! partition_exists "${BOOT_DEVICE}" && ! partition_exists "${ROOT_DEVICE}"; then
    parted ${DISK} -- mklabel gpt
    parted ${DISK} -- mkpart ESP fat32 1MB 512MB
    parted ${DISK} -- mkpart root ext2 512MB 100%
    parted ${DISK} -- set 1 esp on
    echo "partition finished"

    ensure_partition_devices

    mkfs.fat -F 32 -n boot "${BOOT_DEVICE}"
    mkfs.ext2 -L nixos "${ROOT_DEVICE}"
    echo "mkfs finished"
else
    ensure_partition_devices
    echo "Partitions ${BOOT_DEVICE} and ${ROOT_DEVICE} already exist — skipping partitioning and mkfs"
fi

if findmnt -M ${BUILD_DIR}/boot >/dev/null; then
	umount -d ${BUILD_DIR}/boot
fi
if findmnt -M ${BUILD_DIR} >/dev/null; then
	umount -d ${BUILD_DIR}
fi

mkdir -p ${BUILD_DIR}
mount -o sync,dirsync "${ROOT_DEVICE}" ${BUILD_DIR}

mkdir -p ${BUILD_DIR}/boot
mkdir -p ${BUILD_DIR}/etc/nixos
mount -o umask=077,sync,dirsync "${BOOT_DEVICE}" ${BUILD_DIR}/boot

echo "${BUILD_DIR} is mounted successfully!"

cleanup() {
    umount -d ${BUILD_DIR}/boot 2>/dev/null || true
    umount -d ${BUILD_DIR} 2>/dev/null || true
    rm -rf ${BUILD_DIR}
}
trap cleanup EXIT INT TERM ERR

cp $CONFIG_PATH ${BUILD_DIR}/etc/nixos/configuration.nix
cp @aster-configuration@ ${BUILD_DIR}/etc/nixos/aster_configuration.nix
cp -r @aster-etc-nixos@/modules ${BUILD_DIR}/etc/nixos
cp -r @aster-etc-nixos@/overlays ${BUILD_DIR}/etc/nixos

COMMON_NIX_ARGS=(
    --option extra-substituters "@aster-substituters@"
    --option extra-trusted-public-keys "@aster-trusted-public-keys@"
)

# Pre-build the NixOS system closure on the host
# so that the result is cached in the host's Nix store.
# Without this, nixos-install would build the system inside the (possibly empty) target image,
# causing redundant downloads after every `make clean`.
#
# See: https://www.mankier.com/8/nixos-install#--system
SYSTEM_CLOSURE=$(nix-build '<nixpkgs/nixos>' -A system \
    -I "nixos-config=${BUILD_DIR}/etc/nixos/configuration.nix" \
    "${COMMON_NIX_ARGS[@]}" \
    --no-out-link)

export PATH=${PATH}:/run/current-system/sw/bin
nixos-install --root ${BUILD_DIR} --no-root-passwd \
    --system "${SYSTEM_CLOSURE}" \
    "${COMMON_NIX_ARGS[@]}"

echo "Congratulations! Asterinas NixOS has been installed successfully!"
