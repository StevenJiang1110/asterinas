Kata helpers live here.

- `common.sh`: shared shell helpers used by the other Kata scripts
- `run_kata.sh`: provides predefined `smoke`, `pass`, and `workload` Kata tasks
- `kata_env.sh`: provides `install` and `check` for the shared Kata environment lifecycle
- `kata_services.sh`: provides `start`, `stop`, and `status` for the background Kata smoke-test services
- `config/`: repo-owned Kata, CNI, `containerd`, and smoke-test config files used by the scripts

## Configuration

- Default smoke-test settings live in `tools/kata/config/smoke-test.env`.
- Override a value ad hoc with environment variables, for example:
  `KATA_TEST_IMAGE=docker.io/library/ubuntu:24.04 bash tools/kata/run_kata.sh smoke`
- Or point `KATA_CONFIG_FILE` at another Bash config fragment.
- Legacy `KATA_ALPINE_*` overrides still map to the new `KATA_TEST_*` names.
- The default workload still pulls Alpine and runs `cat /etc/alpine-release`, but
  the image, in-container command, and output check are now script-configurable.

## Local virtio-fs note

- The local `bash tools/kata/run_kata.sh smoke` flow now uses `virtio-fs`.
- In the current dev container, `/dev/shm` is only `64M`, which is too small
  for Kata's default shared guest memory backend when the VM memory is `2048M`.
- The repo-owned Kata drop-in therefore sets `file_mem_backend = "/tmp"` so
  `virtio-fs` local runs do not fail during early VM boot.
- If local `virtio-fs` bring-up fails again, check both:
  - the outer container flags (`--privileged --cgroupns=host`)
  - the available space of the configured file-backed memory directory
