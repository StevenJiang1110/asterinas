# Kata virtio-fs Debug Log

## Goal

Switch the local `bash tools/kata/run_kata.sh smoke` flow from `virtio-9p` to
`virtio-fs`, then find
whether the resulting failure is caused by repo configuration, the local dev
container environment, or Kata/QEMU/runtime behavior.

## Current Change

- Changed `tools/kata/config/kata-10-container.toml`:
  - From `shared_fs = "virtio-9p"`
  - To `shared_fs = "virtio-fs"`

## Initial Verification

- `bash tools/kata/run_kata.sh smoke` installs
  `tools/kata/config/kata-10-container.toml` as
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
bash tools/kata/run_kata.sh smoke
```

Result:

- `bash tools/kata/run_kata.sh smoke` fails after switching to `virtio-fs`.
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
- Next step: rerun `bash tools/kata/run_kata.sh smoke` and verify whether the
  QEMU command line now uses `/tmp` and whether the Kata workload succeeds.

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
bash tools/kata/run_kata.sh smoke
KATA_PASSES=2 bash tools/kata/run_kata.sh smoke
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

## `jjf-dev/kata-containers` Release Attempt

### Target Release

- Release page:
  `https://github.com/jjf-dev/kata-containers/releases/tag/3.28.0-20260414-asterinas`
- Key asset:
  `kata-static-3.28.0-asterinas-amd64.tar.zst`

### Release Layout Findings

- The release manifest says it is built on top of official Kata `3.28.0`.
- The release summary shows it contains:
  - the normal Linux guest image files
  - `aster-kernel-osdk-bin.qemu_elf`
  - `configuration-asterinas.toml`
  - `configuration.toml -> configuration-asterinas.toml`
  - `vmlinux.container -> aster-kernel-osdk-bin.qemu_elf`

### Repo Changes For This Attempt

- Added support for installing Kata from a static release tarball URL.
- Added optional support for a local Asterinas kernel overlay path
  (`KATA_ASTERINAS_KERNEL_PATH`) that mirrors the release workflow logic.
- Made the Asterinas kernel overlay opt-in, not the default.
- Allowed `KATA_STATIC_TARBALL_URL` to be explicitly set to an empty string so
  the helper can fall back to the official payload image.

### Asterinas Kernel Result

- Built local kernel artifact with:

```bash
make kernel BOOT_METHOD=qemu-direct
```

- Applied the same Asterinas overlay pattern used by the release workflow:
  - `configuration.toml -> configuration-asterinas.toml`
  - `vmlinux.container -> aster-kernel-osdk-bin.qemu_elf`
  - `-kernel /opt/kata/share/kata-containers/aster-kernel-osdk-bin.qemu_elf`
  - `-initrd /opt/kata/share/kata-containers/kata-alpine-3.22.initrd`

- Result:
  - QEMU starts
  - VM starts
  - Kata agent never becomes reachable
  - `nerdctl run` fails with:
    - `timed out connecting to vsock <cid>:1024`

- The local reproduction therefore indicates that the Asterinas-kernel release
  flow is not yet sufficient for a working Kata container launch here.

### Linux Image Result

- Disabled the Asterinas kernel overlay and switched back to the Linux image
  path:
  - `configuration.toml -> configuration-qemu.toml`
  - `vmlinux.container -> vmlinux-6.18.15-186`

- Verified with:

```bash
KATA_STATIC_TARBALL_URL= bash tools/kata/run_kata.sh smoke
```

- Result:
  - `bash tools/kata/run_kata.sh smoke` succeeds
  - `cat /etc/alpine-release` returns `3.10.2`

### Current Practical Status

- `virtio-fs` itself is fixed and works.
- The Linux image path works.
- The Asterinas-kernel overlay that mirrors the `jjf-dev/kata-containers`
  release workflow currently does not complete the Kata agent handshake.

## Static Release Cache Change

### Goal

- Avoid re-downloading the static Kata release tarball on every run.
- Keep the downloaded tarball in the current environment.
- Only re-download when the release hash changes.

### Implementation

- Added static tarball cache support in `tools/kata/kata_env.sh install`.
- The helper now:
  - derives or reads a checksum source for the static tarball
  - resolves the current expected SHA256
  - stores the tarball under `KATA_STATIC_TARBALL_CACHE_DIR`
  - verifies the cached file against the expected SHA256
  - reuses the cached file when the hash matches
  - re-downloads only when the hash differs or the cache is invalid

New environment knobs:

- `KATA_STATIC_TARBALL_SHA256_URL`
- `KATA_STATIC_TARBALL_SHA256`
- `KATA_STATIC_TARBALL_CACHE_DIR`

### Cache Validation

Used a local miniature static tarball server to validate the cache logic with
the exact same installer path:

1. First install:
   - downloads the tarball
   - caches it
2. Second install after removing `/opt/kata`:
   - reuses the cached tarball
   - does not fetch the tarball again
3. Third install after changing the served tarball and SHA256:
   - detects the hash change
   - downloads the tarball again

Observed request counts from the local test server:

- First run tarball GET count: `1`
- Second run tarball GET count: `1`
- Third run tarball GET count: `2`

This confirms:

- no redundant re-download when the hash is unchanged
- automatic re-download after the hash changes

### Real Environment Verification

After the cache logic change, restored the real runnable setup and verified:

```bash
KATA_STATIC_TARBALL_URL= bash tools/kata/run_kata.sh smoke
```

Result:

- `bash tools/kata/run_kata.sh smoke` succeeds
- `cat /etc/alpine-release` returns `3.10.2`
