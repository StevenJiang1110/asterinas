# Kata CI Handoff

## Status

- Current refactor branch: `kata-ci-split-steps`
- Based on branch: `kata-ci`
- Previous PR: `https://github.com/StevenJiang1110/asterinas/pull/57`
- Current split PR: `https://github.com/StevenJiang1110/asterinas/pull/58`
- Workflow: `.github/workflows/test_kata_guest_os.yml`
- Latest repo commit in this handoff state:
  - current `HEAD` after the Kata script-entrypoint cleanup
- Latest passing run before the 3-step split: `29`
  - `https://github.com/StevenJiang1110/asterinas/actions/runs/24171125156`
- Latest passing commit before the 3-step split:
  - `d1e190b97` `Reorganize Kata CI tools`
- Current refactor status:
  - workflow split into install plus two full start / check / test / cleanup passes
  - helper scripts are now named for shared local and workflow use:
    `kata_env.sh`, `kata_services.sh`, and `run_kata.sh`
  - `run_kata.sh` now provides the shared `smoke`, `pass`, and `workload`
    tasks for both workflow and local entrypoints
  - `kata_env.sh` now provides the shared `install` and `check` environment
    tasks; repo-owned configs live under `tools/kata/config/`
  - the top-level local entrypoint is now `bash tools/kata/run_kata.sh smoke`;
    the old top-level `make kata` wrapper has been removed
  - the smoke workload is now configured from `tools/kata/config/smoke-test.env`
    instead of being hard-coded in the entry script
  - the workload-facing script names are now consistently `run_kata_*`
  - the main smoke-test environment knobs now use the generic `KATA_TEST_*`
    naming, with compatibility mapping from the old `KATA_ALPINE_*` names
  - the workflow now takes its common defaults from
    `tools/kata/config/smoke-test.env` and only sets
    `KATA_CGROUP_NAMESPACE=host` explicitly
  - `kata_env.sh install` now skips `apt-get update` / `apt-get install`
    when the required distro packages are already present, unless
    `KATA_FORCE_APT=1` is set
  - the main shell helpers now provide `--help` output and small inline
    comments for easier local use
  - successful local runs are quieter now; verbose `kata-runtime check` and
    `nerdctl --debug-full` output stay in files by default and can be
    re-enabled with `KATA_CHECK_DEBUG=1` and `KATA_NERDCTL_DEBUG=1`
  - latest passing run that validates the two-pass workflow logic: `24180005806`
  - `https://github.com/StevenJiang1110/asterinas/actions/runs/24180005806`
- handoff-only commits may trigger newer reruns without changing the workflow logic

The original goal is complete:

- run a real Kata workload in GitHub Actions
- use `nerdctl` with Kata
- start `alpine`
- verify `cat /etc/alpine-release`

## Final Design

The working setup uses a GitHub Actions job container:

- image: `asterinas/asterinas:0.17.1-20260319`
- container options:
  - `--privileged`
  - `--cgroupns host`

Inside that job container, the workflow:

- installs `containerd`, `nerdctl`, `crictl`, Kata artifacts, and related packages
- runs `tools/kata/kata_env.sh install` once
- runs `tools/kata/run_kata.sh pass` twice
- prints the key failure logs directly in the job output

## Key Files

- Workflow: `.github/workflows/test_kata_guest_os.yml`
- Local entrypoint: `tools/kata/run_kata.sh smoke`
- Shared task runner: `tools/kata/run_kata.sh`
- Environment helper: `tools/kata/kata_env.sh`
- Service helper: `tools/kata/kata_services.sh`
- Config directory: `tools/kata/config/`

## Verified Findings

- `container:` jobs do work for this CI path.
- Manual outer `docker run` is not required.
- The only validated outer-container requirements are:
  - `--privileged`
  - `--cgroupns host`
- These outer flags were verified as not required:
  - `--network host`
  - `--device /dev/kvm`
  - `--device /dev/vhost-vsock`
  - `-v /dev:/dev`
  - `--security-opt apparmor=unconfined`
  - `--security-opt seccomp=unconfined`
- `kata-runtime check` is diagnostic only for this CI path; the hard gate is the
  configured `nerdctl` Kata workload.
- The old `.kata-ci-diagnostics` artifact upload was only for bring-up and is
  not part of the steady-state workflow anymore.
- The inner `containerd` / `nerdctl` stack must use snapshotter `native`.

Validation runs:

- no outer `--network host`:
  `https://github.com/StevenJiang1110/asterinas/actions/runs/24169398907`
- outer-flag matrix run 1:
  `https://github.com/StevenJiang1110/asterinas/actions/runs/24169773210`
- outer-flag matrix run 2:
  `https://github.com/StevenJiang1110/asterinas/actions/runs/24170147179`
- migrated back to `container:`:
  `https://github.com/StevenJiang1110/asterinas/actions/runs/24170728737`

## If It Breaks

Check these first:

1. Did the workflow lose either required job-container option?
   - `--privileged`
   - `--cgroupns host`
2. Did the inner `containerd` / `nerdctl` snapshotter stop using `native`?
3. Do the job logs still show the key failure output:
   - `kata-check.txt`
   - `containerd.log`
   - `nerdctl-run-command.txt`

## Notes

- PR `#58` reflects the two-pass Kata workflow validation run at
  `https://github.com/StevenJiang1110/asterinas/actions/runs/24180005806`.
- As of April 10, 2026, the repo has since been cleaned up so that local and
  workflow paths share the same helper naming and the same single-pass entry
  script.

## Local Kata Script Handoff

As of April 15, 2026:

- the repo has `bash tools/kata/run_kata.sh smoke`
- the local scripts are wired up
- the current blocker is no longer repo logic

### Local Changes

- Added `tools/kata/run_kata.sh`
- Removed the top-level `make kata` wrapper so local runs go directly through
  `tools/kata/`
- Local Kata script runs now use `virtio-fs`
- Local Kata config now sets `file_mem_backend = "/tmp"` to avoid the default
  `virtio-fs` shared guest memory backend path (`/dev/shm`), which was only
  `64M` in this dev container
- Local install now reuses the official
  `quay.io/kata-containers/kata-deploy:${KATA_VERSION}` payload image
- Local smoke test now pulls Alpine from `quay.io/libpod/alpine:latest`
  because `docker.io` was not reachable in this environment
- Local install no longer runs `apt-get update` on every local smoke run when the
  required distro packages are already installed
- Successful local runs no longer print the full `kata-runtime check -v` and
  `nerdctl --debug-full` output by default
- The smoke image / command / output check now live in
  `tools/kata/config/smoke-test.env`
- The main configurable workload knobs now use `KATA_TEST_*`; the old
  `KATA_ALPINE_*` names are compatibility aliases only

### Local Verification Result

On April 10, 2026, after restarting the dev container with `--cgroupns=host`,
the local flow was re-verified successfully.

On April 14, 2026, the local flow was re-verified again after switching the
repo-owned Kata drop-in from `virtio-9p` to `virtio-fs` and explicitly setting
`file_mem_backend = "/tmp"`:

- Kata install works
- inner `containerd` starts
- `bash tools/kata/kata_env.sh check` passes
- `/dev/kvm` is usable
- `nerdctl pull` succeeds
- `nerdctl run --runtime io.containerd.kata.v2 ...` succeeds
- `bash tools/kata/run_kata.sh smoke` succeeds
- `KATA_PASSES=2 bash tools/kata/run_kata.sh smoke` succeeds
- `kata-runtime env` reports `SharedFS = "virtio-fs"`
- repeated local smoke runs stay quiet and do not re-run package refresh by
  default

### Current Status

The earlier local cgroup failure was caused by the outer dev container
environment, not by the repo helper scripts.

The later local `virtio-fs` failure was caused by the dev container's default
`/dev/shm` size, not by Kata's `virtio-fs` support itself. The repo-side fix is
to keep `virtio-fs` enabled and point `file_mem_backend` at a directory with
enough space.

With the dev container started using `--privileged --cgroupns=host`, the
current repo-side Kata workflow and local scripts both work.

### What To Do Next

If Kata local bring-up fails again, first verify that the outer dev container
still includes the same cgroup setup used by the GitHub Actions job:

- `--privileged`
- `--cgroupns=host`

Then verify that the `virtio-fs` guest memory backing directory has enough
space. In this environment, `/dev/shm` was only `64M`, so the local drop-in
uses `file_mem_backend = "/tmp"` instead.

The current repo-side handoff point is:

- `bash tools/kata/run_kata.sh smoke` is the top-level local entrypoint
- workflow and local runs share the same unified task runner
- the helper names are no longer CI-specific
- the smoke workload is configured from `tools/kata/config/smoke-test.env`
- the workload-facing script names are consistently `run_kata_*`
- the main configurable workload knobs are `KATA_TEST_*`
- the main shell helpers provide `--help`
- repeated local runs avoid unnecessary package refresh
- successful local runs keep most debug logs out of the terminal by default
