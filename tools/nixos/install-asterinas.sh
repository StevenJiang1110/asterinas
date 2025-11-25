#!/bin/bash

# SPDX-License-Identifier: MPL-2.0

set -e

NIXOS_DIR=$(realpath $1)
SCRIPT_DIR=$(cd "$( dirname "${BASH_SOURCE[0]}" )" && pwd)
ASTER_IMAGE_PATH=${NIXOS_DIR}/asterinas.img
INSTALL_DIR=${NIXOS_DIR}/nixos_rootfs
ASTERINAS_DIR=$(realpath ${SCRIPT_DIR}/../..)
DISTRO_DIR=$(realpath ${ASTERINAS_DIR}/distro)
NIXOS_DISK_SIZE_IN_MB=${NIXOS_DISK_SIZE_IN_MB:-"8196"}
LOG_LEVEL=${LOG_LEVEL:-"error"}

echo "************  NIXOS SETTINGS:  ************"
echo "DISK_SIZE: ${NIXOS_DISK_SIZE_IN_MB}MB"
echo "INSTALL_DIR=${INSTALL_DIR}"
echo "BUILD_IMAGE_PATH=${ASTER_IMAGE_PATH}"
echo "CONFIGURATION=${DISTRO_DIR}/configuration.nix"
echo "LOG_LEVEL=${LOG_LEVEL}"
echo "STAGE_2_INIT=${NIXOS_STAGE_2_INIT}"
echo "STAGE_2_ARGS=${NIXOS_STAGE_2_ARGS}"
echo "************END OF NIXOS SETTINGS************"

get_top_level_dir() {
    local input_path="$1"
    local temp_path
    local first_component
    local top_level_dir

    # Remove leading slash
    temp_path="${input_path#/}"

    # Extract the first path component
    # If the path is "/", temp_path will be an empty string
    first_component=$(echo "$temp_path" | cut -d'/' -f1)

    # Construct the top-level directory
    if [[ -n "$first_component" ]]; then
        top_level_dir="/$first_component"
    else
        # If the path itself is "/", or the input is an empty string
        top_level_dir="/"
    fi

    echo "$top_level_dir"
}

mkdir -p ${NIXOS_DIR}
cp -rL ${ASTERINAS_DIR}/test/build/initramfs/etc/resolv.conf ${NIXOS_DIR}

if [ ! -e ${ASTER_IMAGE_PATH} ]; then
    dd if=/dev/zero of=${ASTER_IMAGE_PATH} bs=1M count=${NIXOS_DISK_SIZE_IN_MB}
fi

DEVICE=$(losetup -fP --show ${ASTER_IMAGE_PATH})
echo "${DEVICE} created"

if [ ! -b "${DEVICE}p1" ] && [ ! -b "${DEVICE}p2" ]; then
    parted ${DEVICE} -- mklabel gpt
    parted ${DEVICE} -- mkpart ESP fat32 1MB 512MB
    parted ${DEVICE} -- mkpart root ext2 512MB 100%
    parted ${DEVICE} -- set 1 esp on
    echo "partition finished"

    mkfs.fat -F 32 -n boot "${DEVICE}p1"
    mkfs.ext2 -L nixos "${DEVICE}p2"
    echo "mkfs finished"
else
    echo "Partitions ${DEVICE}p1 and ${DEVICE}p2 already exist — skipping partitioning and mkfs"
fi

if findmnt -M ${INSTALL_DIR}/boot >/dev/null; then
	umount -d ${INSTALL_DIR}/boot
fi
if findmnt -M ${INSTALL_DIR} >/dev/null; then
	umount -d ${INSTALL_DIR}
fi

mkdir -p ${INSTALL_DIR}
mount -o sync,dirsync "${DEVICE}p2" ${INSTALL_DIR}

mkdir -p ${INSTALL_DIR}/boot
mkdir -p ${INSTALL_DIR}/etc/nixos
mount -o umask=077,sync,dirsync "${DEVICE}p1" ${INSTALL_DIR}/boot

echo "mount finished"

cp ${DISTRO_DIR}/configuration.nix ${INSTALL_DIR}/etc/nixos
cp -r ${DISTRO_DIR}/overlays ${INSTALL_DIR}/etc/nixos

top_level_dir=$(get_top_level_dir ${INSTALL_DIR})
chmod o+rx ${top_level_dir}
nixos-install --root ${INSTALL_DIR} --no-root-passwd

umount -d ${INSTALL_DIR}/boot
umount -d ${INSTALL_DIR}
rm -rf ${INSTALL_DIR}

losetup -d $DEVICE

echo "Install NixOS succeeds!"