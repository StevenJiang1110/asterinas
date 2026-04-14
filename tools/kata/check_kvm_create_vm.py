#!/usr/bin/env python3

import errno
import fcntl
import os
import sys


KVM_DEVICE_PATH = "/dev/kvm"
KVM_IOC_GROUP = 0xAE
KVM_CREATE_VM = (KVM_IOC_GROUP << 8) | 0x01


def main() -> int:
    try:
        device_status = os.stat(KVM_DEVICE_PATH)
    except OSError as error:
        print(f"Failed to stat {KVM_DEVICE_PATH}: {error}", file=sys.stderr)
        return 1

    print(f"Inspecting {KVM_DEVICE_PATH}")
    print(
        "mode={mode} uid={uid} gid={gid} rdev={major}:{minor}".format(
            mode=oct(device_status.st_mode),
            uid=device_status.st_uid,
            gid=device_status.st_gid,
            major=os.major(device_status.st_rdev),
            minor=os.minor(device_status.st_rdev),
        )
    )

    try:
        kvm_fd = os.open(KVM_DEVICE_PATH, os.O_RDWR | os.O_CLOEXEC)
    except OSError as error:
        print(f"Failed to open {KVM_DEVICE_PATH}: {error}", file=sys.stderr)
        return 1

    print(f"Opened {KVM_DEVICE_PATH} successfully")

    vm_fd = None
    try:
        vm_fd = fcntl.ioctl(kvm_fd, KVM_CREATE_VM, 0)
    except OSError as error:
        error_name = errno.errorcode.get(error.errno, "UNKNOWN")
        print(
            "KVM_CREATE_VM ioctl failed: errno={errno_value} ({error_name}): {message}".format(
                errno_value=error.errno,
                error_name=error_name,
                message=error.strerror,
            ),
            file=sys.stderr,
        )
        return 1
    finally:
        if isinstance(vm_fd, int) and vm_fd >= 0:
            os.close(vm_fd)
        os.close(kvm_fd)

    print(f"KVM_CREATE_VM ioctl succeeded: vm_fd={vm_fd}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
