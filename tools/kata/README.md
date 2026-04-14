Kata helpers live here.

- `common.sh`: shared shell helpers used by the other Kata scripts
- `run_kata_alpine.sh`: installs the local test environment and runs one or more full Alpine smoke-test passes
- `run_kata_pass.sh`: runs one full Kata pass (`start` + `check` + `test` + cleanup) for both local and workflow entrypoints
- `install_kata_env.sh`: installs the distro packages, `nerdctl`, and Kata payload used by the shared Kata smoke test
- `start_kata_services.sh`: installs repo-owned configs, prepares host prerequisites, and starts the background services used by the smoke test
- `check_kata_env.sh`: verifies that the Kata and `containerd` environment is ready
- `test_nerdctl_alpine.sh`: runs the `nerdctl` + Kata Alpine smoke test
- `stop_kata_services.sh`: stops the background services started by the Kata helpers
- `check_kvm_create_vm.py`: probes `KVM_CREATE_VM` directly for diagnostics
- `config/`: repo-owned Kata, CNI, and `containerd` config templates used by the scripts
