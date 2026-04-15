Kata helpers live here.

- `common.sh`: shared shell helpers used by the other Kata scripts
- `run_kata_smoke.sh`: installs the local test environment and runs one or more configurable smoke-test passes
- `run_kata_pass.sh`: runs one full Kata pass (`start` + `check` + `test` + cleanup) for both local and workflow entrypoints
- `run_kata_workload.sh`: runs the configurable Kata workload with `nerdctl`
- `install_kata_env.sh`: installs the distro packages, `nerdctl`, and Kata payload used by the shared Kata smoke test
- `start_kata_services.sh`: installs repo-owned configs, prepares host prerequisites, and starts the background services used by the smoke test
- `check_kata_env.sh`: verifies that the Kata and `containerd` environment is ready
- `stop_kata_services.sh`: stops the background services started by the Kata helpers
- `check_kvm_create_vm.py`: probes `KVM_CREATE_VM` directly for diagnostics
- `config/`: repo-owned Kata, CNI, `containerd`, and smoke-test config files used by the scripts

## Configuration

- Default smoke-test settings live in `tools/kata/config/smoke-test.env`.
- Override a value ad hoc with environment variables, for example:
  `KATA_TEST_IMAGE=docker.io/library/ubuntu:24.04 bash tools/kata/run_kata_smoke.sh`
- Or point `KATA_CONFIG_FILE` at another Bash config fragment.
- Legacy `KATA_ALPINE_*` overrides still map to the new `KATA_TEST_*` names.
- The default workload still pulls Alpine and runs `cat /etc/alpine-release`, but
  the image, in-container command, and output check are now script-configurable.

## Local virtio-fs note

- The local `bash tools/kata/run_kata_smoke.sh` flow now uses `virtio-fs`.
- In the current dev container, `/dev/shm` is only `64M`, which is too small
  for Kata's default shared guest memory backend when the VM memory is `2048M`.
- The repo-owned Kata drop-in therefore sets `file_mem_backend = "/tmp"` so
  `virtio-fs` local runs do not fail during early VM boot.
- If local `virtio-fs` bring-up fails again, check both:
  - the outer container flags (`--privileged --cgroupns=host`)
  - the available space of the configured file-backed memory directory
