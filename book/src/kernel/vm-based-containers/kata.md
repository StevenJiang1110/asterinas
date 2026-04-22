# Using Asterinas as a Kata Guest Kernel

This guide explains how to use Asterinas
as the guest kernel for Kata Containers.

[Kata Containers](https://github.com/kata-containers/kata-containers)
is a VM-based container runtime.
It runs each container inside its own virtual machine,
so containers do not share the host kernel.
This design combines the deployment model of containers
with the stronger isolation boundary of virtual machines.
It reduces the risk that a compromised workload can affect
the host or other containers.

## Prepare the host

Kata Containers requires a host with KVM and vhost support.
The commands in this guide are currently written for x86_64 hosts.

Verify that the required device nodes exist:

```bash
ls /dev/kvm /dev/vhost-net /dev/vhost-vsock
```

If any of them are missing,
load the matching kernel modules:

```bash
sudo modprobe kvm
sudo modprobe kvm_intel  # Or use kvm_amd on AMD hosts.
sudo modprobe vhost_net
sudo modprobe vhost_vsock
```

Then make sure the user running Docker can access
`/dev/kvm`, `/dev/vhost-net`, and `/dev/vhost-vsock`.

## Enter the Kata environment

Choose one of the two workflows below.
End users should use the prebuilt `asterinas/kata` image.
Kernel developers should use the `asterinas/asterinas` image
with source mounts.

### Shared Docker arguments

All workflows need the same Docker access to KVM, vhost, cgroups,
and temporary storage.
Define the shared Docker arguments first:

```bash
KATA_DOCKER_ARGS=(
    --cgroupns host
    --privileged
    --device /dev/kvm
    --device /dev/vhost-net
    --device /dev/vhost-vsock
    --tmpfs /tmp:exec,mode=1777,size=8g
    --tmpfs /var/lib/containerd:exec,mode=755,size=8g
)
```

These flags are required for Kata:

- `--cgroupns host` shares the host cgroup namespace
  so that `containerd` inside the container can manage Kata workloads.
- `--privileged` is required for KVM and nested container management.
- `--device /dev/kvm`, `--device /dev/vhost-net`,
  and `--device /dev/vhost-vsock` expose the virtualization devices
  that Kata needs.
- `--tmpfs /tmp:exec,...` and `--tmpfs /var/lib/containerd:exec,...`
  give Kata enough temporary storage space to create containers.

### For end users

Use the `asterinas/kata` image.
It already includes Kata, the Asterinas guest kernel,
the Kata configuration,
and the `tools/kata/` helper scripts in `/root/kata-containers`.
The command below makes `/root/kata-containers` the working directory:

```bash
docker run -it \
    "${KATA_DOCKER_ARGS[@]}" \
    -w /root/kata-containers \
    asterinas/kata:0.17.2-20260407
```

After entering the container,
continue with [Start a Kata workload](#start-a-kata-workload).

### For kernel developers

Kernel developers usually use the `asterinas/asterinas` image
as the Asterinas development environment.
This workflow lets you rebuild the kernel
and test how kernel changes affect Kata workloads.
Mount both source trees:

- The Asterinas source tree is mounted at `/root/asterinas`.
- The Asterinas fork of Kata Containers is mounted at `/root/kata-containers`.

The Kata fork carries Asterinas-specific patches, helper scripts,
and configuration for building, installing, and testing Kata
with Asterinas as the guest kernel.
Use the Asterinas fork for changes to the Asterinas guest-kernel integration
until those patches, scripts, and configuration are upstreamed.
The command below makes `/root/kata-containers` the working directory,
so the `tools/kata/` helper scripts are available
at the relative paths used below.

```bash
# Assumes the Asterinas source has already been cloned locally.
ASTERINAS_SRC=$HOME/asterinas

# Clones the Asterinas fork of kata-containers locally as well.
cd $HOME && git clone https://github.com/asterinas/kata-containers.git
KATA_SRC=$HOME/kata-containers

docker run -it \
    "${KATA_DOCKER_ARGS[@]}" \
    -v "${ASTERINAS_SRC}:/root/asterinas" \
    -v "${KATA_SRC}:/root/kata-containers" \
    -w /root/kata-containers \
    asterinas/asterinas:0.17.2-20260407
```

After entering the `asterinas/asterinas` image,
install the Kata dependencies and configuration:

```bash
tools/kata/kata_env.sh install
```

Then continue with [Start a Kata workload](#start-a-kata-workload).

## Start a Kata workload

### Start services

Start the background services required by Kata.
At the moment,
these services are `containerd` and `syslogd`:

```bash
tools/kata/kata_services.sh start
```

Check their status to make sure they started successfully:

```bash
tools/kata/kata_services.sh status
```

### Run Alpine with Kata

Then use `nerdctl` with Kata to start an Alpine container:

```bash
nerdctl run \
    --cgroup-manager cgroupfs \
    --net none \
    --runtime io.containerd.kata.v2 \
    --name foo \
    -it \
    docker.io/alpine:latest
```

The `nerdctl` flags above are also required:

- `--cgroup-manager cgroupfs` is required because the example runs
  inside a Docker container where the systemd cgroup driver
  is not available.
- `--net none` is required because Asterinas does not yet support
  hot-plugged network devices,
  so workloads inside the guest cannot access the network.

### Verify the guest

After the container starts,
you are inside Alpine.
Run the following commands to confirm that the workload
is running inside an Asterinas guest, not the host kernel:

```bash
cat /proc/cmdline
cat /etc/alpine-release
```

The `/proc/cmdline` output should contain the Asterinas kernel image path.
For the prebuilt `asterinas/kata` image,
look for:

```text
/opt/kata/share/kata-containers/aster-kernel-osdk-bin.qemu_elf
```

For a locally built kernel,
look for:

```text
/root/asterinas/target/osdk/aster-kernel-osdk-bin.qemu_elf
```

This is the most direct check that Kata booted an Asterinas guest kernel.
The `/etc/alpine-release` output should print the Alpine version
of the container rootfs,
confirming that you are inside the Alpine workload.

### Clean up

After exiting the container,
remove it with:

```bash
nerdctl rm foo
```

## Use a local kernel

You can also point Kata to a locally built guest kernel.
This workflow assumes that you entered the Kata environment
through the [For kernel developers](#for-kernel-developers) flow above.
Build the kernel first:

```bash
cd /root/asterinas && make kernel BOOT_METHOD=qemu-direct
```

Verify that the command produced the kernel image:

```bash
ls /root/asterinas/target/osdk/aster-kernel-osdk-bin.qemu_elf
```

Pass that path to `tools/kata/kata_env.sh install`.
This installs `/etc/kata-containers/configuration.toml`
with the local kernel path already configured:

```bash
tools/kata/kata_env.sh install \
    --kernel /root/asterinas/target/osdk/aster-kernel-osdk-bin.qemu_elf
```

Then start the Kata services.
If they are already running from the previous run,
stop them first with:

```bash
tools/kata/kata_services.sh stop
```

Then start the services again:

```bash
tools/kata/kata_services.sh start
tools/kata/kata_services.sh status
```

Then run the `nerdctl run` command from the previous section.
Kata will boot the workload with your local Asterinas kernel.
