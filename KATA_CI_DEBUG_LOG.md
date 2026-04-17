# Kata CI Debug Log

## 2026-04-16T11:12:33Z UTC

- Waiting for jjf-dev/kata-containers run `24506696843` to complete before switching the tarball URL.
- 2026-04-16T11:13Z: upstream run status = in_progress
  - `Build Asterinas kernel` job succeeded
  - `Build and publish Asterinas Kata release` job is still running
  - release publish step has not started yet
- 2026-04-16T11:18Z: added dynamic latest-release resolution
  - default source repo is now `jjf-dev/kata-containers`
  - `kata_env.sh install` can resolve the newest `kata-static-*-asterinas-amd64.tar.zst`
  - future updates no longer need a hard-coded tarball URL in `smoke-test.env`
- 2026-04-16T11:19Z: updated source-of-truth hints
  - workflow now sets `KATA_STATIC_TARBALL_RELEASE_REPO=jjf-dev/kata-containers`
  - README now documents that latest Asterinas Kata tarballs are resolved from that repo by default
- 2026-04-16T11:21Z: upstream latest release changed to `3.28.0-20260416`
  - proceeding to use the newest `jjf-dev/kata-containers` release for the next PR CI run
- 2026-04-16T11:32Z: pushed commit `89559ae9f`
  - switched from hard-coded Kata tarball URL to latest-release resolution
  - next PR CI run should pick `jjf-dev/kata-containers` latest release `3.28.0-20260416-asterinas`
- 2026-04-16T11:40Z: PR run `24507902338` failed with a new symptom
  - previous `virtiofsd` path / hard-coded host log path issue is gone
  - QEMU now starts and the VM reaches `VM started`
  - failure moved to agent startup timeout: `timed out connecting to vsock ...:1024`
  - next action: expose QEMU console and serial logs in CI output to inspect guest boot
- 2026-04-16T11:41Z: pushed commit `6e2f636b8`
  - added CI failure groups for `/tmp/kata-console.log` and `/tmp/kata-qemu-serial.log`
  - goal is to inspect guest boot after the vsock timeout failure
- 2026-04-16T11:56Z: formed a stronger hypothesis from the new release metadata
  - latest release body says the packaged guest initrd should be `/opt/kata/share/kata-containers/kata-containers-initrd.img`
  - but the CI QEMU command is still booting `/opt/kata/share/kata-containers/kata-alpine-3.22.initrd`
  - patched `kata_services.sh` to normalize copied Kata config to the packaged `kata-containers-initrd.img` when it exists
- 2026-04-16T11:57Z: pushed commit `92590493f`
  - normalized copied Kata config to use the packaged `kata-containers-initrd.img`
  - next PR CI run should no longer boot `kata-alpine-3.22.initrd`
- 2026-04-16T12:31Z: PR run `24509770015` failed before Kata startup
  - cause: transient Ubuntu mirror mismatch during `apt-get update`
  - error: `File has unexpected size ... Mirror sync in progress?`
  - fix: add retry logic around `apt-get update` in `kata_env.sh install`
- 2026-04-16T12:32Z: pushed commit `3acab6550`
  - added retry logic for transient `apt-get update` mirror-sync failures
  - next PR CI run includes all current fixes: latest release resolution, packaged initrd normalization, guest log capture, and apt retry
- 2026-04-16T12:48Z: initrd normalization did not take effect in CI
  - observed QEMU still booting `kata-alpine-3.22.initrd`
  - likely cause: the copied TOML line has leading indentation, so the previous `sed` pattern missed it
  - updated the rewrite rule to match optional leading whitespace before `initrd =`
- 2026-04-16T12:49Z: pushed commit `87c84f18c`
  - fixed the initrd rewrite regex so it also matches indented TOML lines

## 2026-04-17T05:15Z UTC

- Confirmed that the Asterinas Kata static release contains both Linux and
  Asterinas guest kernel artifacts.
- Switched CI to the Linux guest kernel:
  - commit `493a5b6a0` `Use Linux guest kernel for Kata CI`
  - workflow sets `KATA_GUEST_KERNEL=linux`
  - `kata_services.sh` selects `configuration-qemu.toml` and points
    `vmlinux.container` / `vmlinuz.container` back to the Linux kernel
  - PR run `24545819068` passed both Kata passes
- Removed the forced `nerdctl --snapshotter native` path:
  - commit `b69774333` `Stop forcing native snapshotter in Kata CI`
  - `KATA_SNAPSHOTTER` now defaults to empty
  - PR run `24546258768` failed in `Run Kata pass 1`
  - failure showed `snapshotter=nerdctl-default`; containerd then used
    `snapshotter=overlayfs` and Kata failed with
    `failed to create shim task: invalid argument`
- Locally reproduced the overlayfs backing issue:
  - overlayfs support exists in the kernel
  - overlay mount fails when `upperdir` and `workdir` are under the outer
    container's overlay-backed `/tmp`
  - the same overlay mount succeeds when `upperdir` and `workdir` are on a
    fresh `tmpfs`
  - conclusion: the problem is nested overlay upper/work backing, not guest
    `virtio-fs`
- Added an overlayfs preflight helper:
  - commit `42cfd4f13` `Use tmpfs for Kata CI overlay staging`
  - new script: `tools/kata/check_overlayfs.sh`
  - README documents how to run it locally
- Updated CI container options to put host-side overlay staging on `tmpfs`:
  - `--tmpfs /tmp:exec,mode=1777,size=8g`
  - `--tmpfs /var/lib/containerd:exec,mode=755,size=8g`
  - commit `83d574b5b` fixed the mount-inspection command
- First tmpfs CI attempt `24548532131` failed during install because Ubuntu
  mirrors timed out; this was unrelated to overlayfs.
- Rerun of `24548532131` passed:
  - `/tmp` and `/var/lib/containerd` both reported as `tmpfs`
  - `tools/kata/check_overlayfs.sh` printed
    `RESULT: current backing filesystem supports overlay upper/work.`
  - both `Run Kata pass 1` and `Run Kata pass 2` succeeded
  - `cat /etc/alpine-release` returned `3.10.2` in both passes
