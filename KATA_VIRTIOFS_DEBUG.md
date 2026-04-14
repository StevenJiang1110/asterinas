# Kata virtio-fs Debug Log

## Goal

Switch the local `make kata` flow from `virtio-9p` to `virtio-fs`, then find
whether the resulting failure is caused by repo configuration, the local dev
container environment, or Kata/QEMU/runtime behavior.

## Current Change

- Changed `tools/kata/config/kata-10-container.toml`:
  - From `shared_fs = "virtio-9p"`
  - To `shared_fs = "virtio-fs"`

## Initial Verification

- `make kata` installs `tools/kata/config/kata-10-container.toml` as
  `/etc/kata-containers/config.d/10-container.toml`.
- After the change, `kata-runtime env` reports:
  - `SharedFS = "virtio-fs"`
  - `VirtioFSDaemon = "/opt/kata/libexec/virtiofsd"`
- `containerd` logs show that the runtime starts `virtiofsd`.
- QEMU is launched with:
  - `-device vhost-user-fs-pci,...,tag=kataShared,queue-size=1024`
  - `-object memory-backend-file,id=dimm1,size=2048M,mem-path=/dev/shm,share=on`

## First Failure

Command:

```bash
make kata
```

Result:

- `make kata` fails after switching to `virtio-fs`.
- `nerdctl` reports:
  - `failed to create shim task`
  - `timed out connecting to vsock <cid>:1024`
- Earlier in `/tmp/containerd.log`, QEMU reports:
  - `error: kvm run failed Bad address`
- This indicates the Kata guest VM starts, then fails before the Kata agent is
  reachable over vsock.

## Working Notes

- There is a stale `/etc/kata-containers/config.d/10-ci-container.toml` with
  `shared_fs = "virtio-9p"` in the local environment.
- Despite that stale file, `kata-runtime env` reports `SharedFS = "virtio-fs"`,
  so the active runtime configuration is using `virtio-fs`.

## New Finding: `/dev/shm` Size

- In the current dev container, `/dev/shm` is only `64M`.
- Under `virtio-fs`, Kata automatically enables file-backed guest memory and,
  by default, uses `/dev/shm` as the backing directory.
- In the failing QEMU command line, the guest memory backend is:

```text
-object memory-backend-file,id=dimm1,size=2048M,mem-path=/dev/shm,share=on
```

- The guest memory size is `2048M`, which is far larger than the available
  `64M` tmpfs mounted at `/dev/shm`.
- This is a strong candidate root cause for the `kvm run failed Bad address`
  failure seen only with `virtio-fs`.

## Experiment In Progress

- Added `file_mem_backend = "/tmp"` to
  `tools/kata/config/kata-10-container.toml` so Kata does not use `/dev/shm`
  for the shared guest memory backend during `virtio-fs` runs.
- Next step: rerun `make kata` and verify whether the QEMU command line now
  uses `/tmp` and whether the Kata workload succeeds.

## Final Result

The issue is fixed.

### Effective Fix

Updated `tools/kata/config/kata-10-container.toml` to:

```toml
[hypervisor.qemu]
enable_debug = true
shared_fs = "virtio-fs"
file_mem_backend = "/tmp"
```

### Why This Fix Works

- `virtio-fs` requires shared guest memory.
- Kata automatically uses a file-backed memory backend for this case.
- Without an explicit override, Kata used `/dev/shm`.
- In this dev container, `/dev/shm` is only `64M`, while the guest memory size
  is `2048M`.
- That mismatch caused the VM to fail during early boot, which then surfaced as:
  - `error: kvm run failed Bad address`
  - `timed out connecting to vsock <cid>:1024`
- After forcing `file_mem_backend = "/tmp"`, the QEMU command line changed from:

```text
-object memory-backend-file,id=dimm1,size=2048M,mem-path=/dev/shm,share=on
```

to:

```text
-object memory-backend-file,id=dimm1,size=2048M,mem-path=/tmp,share=on
```

- `/tmp` has sufficient space in this environment, so the VM boots normally and
  the Kata workload succeeds.

## Validation

Successful runs after the fix:

```bash
make kata
KATA_PASSES=2 make kata
```

Observed success signal:

- `cat /etc/alpine-release` inside the Kata container returned `3.10.2`

## Conclusion

- The original `virtio-fs` failure was not caused by the repo's Kata workflow
  logic itself.
- The direct trigger was the default `virtio-fs` guest memory backing path
  (`/dev/shm`) being too small in the current dev container.
- The repo-side fix is to keep `virtio-fs` enabled and explicitly point
  `file_mem_backend` to a backing directory with enough space, here `/tmp`.
