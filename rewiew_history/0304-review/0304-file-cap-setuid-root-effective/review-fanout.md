---
date: 2026-07-02
mode: diff
base: 043ef13c6
head: f5fc357bb
branch: HEAD
title: "Review file capability execve refactor"
---

# Summary

The refactor improves the shape of file-capability parsing by introducing typed revision and flag handling, but it also removes the dedicated kernel-internal xattr access path and splits exec credential calculation from the metadata reads that later apply the UID/GID transition.

Top issues:

- Critical: `execve()` computes capability sets from one inode metadata snapshot, then later re-reads mode/owner before applying the UID transition. A race can leave the final UID and final capabilities derived from different executable states.
- Major: `FileCapabilities::read_from_inode` now uses normal `get_xattr`, so execute-only programs can fail `execve()` because reading `security.capability` requires DAC read permission.
- Major: `clear_file_capability_xattr` now uses normal `remove_xattr`, so internal privilege invalidation can fail under DAC write checks and may leave capabilities behind after writes or ownership changes.

Structurally, restore a narrow no-DAC xattr path for kernel privilege bookkeeping and make exec UID/GID updates and capability computation consume one consistent executable metadata snapshot.

## Maintainability

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 11

> ```diff
> pub const SECURITY_CAPABILITY_XATTR_NAME: &str = "security.capability";
> ```

`rust-type-invariants` (minor): `SECURITY_CAPABILITY_XATTR_NAME` was changed from a typed `XattrName<'static>` into a plain `&str`, so callers have to re-parse a known kernel constant and use `unwrap()` to recover the invariant. That moves a compile-time representation of the namespace/name pairing into repeated runtime assumptions.

**Fix.** Keep the invariant in the type, for example by restoring a crate-visible typed constant and using it directly at call sites: `pub(crate) const SECURITY_CAPABILITY_XATTR_NAME: XattrName<'static> = XattrName::from_known_valid_full_name(XattrNamespace::Security, "security.capability");`.

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
> let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> match inode.remove_xattr(xattr_name) {
> ```

`bug` (major): `clear_file_capability_xattr` is a kernel-internal cleanup after operations that invalidate file privileges, but it now calls `Inode::remove_xattr`, whose ext2 and ramfs implementations enforce `Permission::MAY_WRITE`. An authorized `chown`/`fchown` on a non-writable regular file with `security.capability` can therefore fail while trying to clear the xattr, even though clearing file capabilities should not depend on DAC write permission.

**Fix.** Shared with the other no-DAC xattr comments: restore narrow kernel-internal read/remove helpers for `security.capability` that bypass DAC permission checks while preserving filesystem/type validation, and use them from `FileCapabilities::read_from_inode` and `clear_file_capability_xattr`.

### `kernel/src/process/credentials/file_capabilities.rs` line 46

> ```diff
> let xattr_name =
>     xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> ```

`bug` (major): `FileCapabilities::read_from_inode` now reads `security.capability` through `Inode::get_xattr`, but the ext2 and ramfs implementations first enforce `Permission::MAY_READ`. A valid execute-only file, for example mode `0111` with a file-capability xattr, can be executed but not read; this path will now return a DAC read-permission error before `execve()` can apply the file capabilities.

**Fix.** Shared with the other no-DAC xattr comments: restore narrow kernel-internal read/remove helpers for `security.capability` that bypass DAC permission checks while preserving filesystem/type validation, and use them from `FileCapabilities::read_from_inode` and `clear_file_capability_xattr`.

### `kernel/src/process/credentials/file_capabilities.rs` line 78

> ```diff
> let (expected_len, permitted, inheritable, root_uid) = match revision {
>     VfsCapRevision::V1 => {
>         let Ok(permitted) = CapSet::try_from_lo_hi(read_u32_le(raw_value, 1)?, 0) else {
>             return_errno_with_message!(
>                 Errno::EINVAL,
>                 "file capabilities contain unsupported capability bits"
>             );
>         };
> ```

`dry` (minor): The V1/V2/V3 branches duplicate the same `permitted` and `inheritable` parsing and unsupported-bit error handling, with only the high-word indexes and optional root UID differing. This makes later changes to capability-bit validation or error wording easy to apply to one revision but miss in another.

**Fix.** Factor the repeated conversion into a helper, such as `read_capset(raw_value, lo_index, hi_index) -> Result<CapSet>`, then have each revision branch specify only its expected length, high-word indexes, and root UID.

## Correctness

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
> diff --git a/kernel/src/fs/vfs/fs_apis/xattr.rs b/kernel/src/fs/vfs/fs_apis/xattr.rs
> @@
> -    match inode.remove_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME) {
> +    let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +    match inode.remove_xattr(xattr_name) {
> 
> diff --git a/kernel/src/fs/file/inode_handle.rs b/kernel/src/fs/file/inode_handle.rs
> @@
>          if reader.remain() > 0 {
>              clear_file_capability_xattr(self.path.inode().as_ref())?;
>          }
> ```

`bug` (major): `clear_file_capability_xattr` now removes the capability xattr through `Inode::remove_xattr`, which performs a fresh DAC write-permission check. Internal privilege invalidation must not depend on current path permissions: a process can open a file writable, another process can `chmod` away write permission, and the already-open writer should still be able to write while the kernel clears `security.capability`. With this change, the write path calls `clear_file_capability_xattr` first and can fail with `EACCES` before performing a write that the open file descriptor authorizes.

**Fix.** Shared with the other no-DAC xattr comments: restore narrow kernel-internal read/remove helpers for `security.capability` that bypass DAC permission checks while preserving filesystem/type validation, and use them from `FileCapabilities::read_from_inode` and `clear_file_capability_xattr`; keep syscall `removexattr` on the permission-checked path.

### `kernel/src/process/credentials/file_capabilities.rs` line 46

> ```diff
> diff --git a/kernel/src/process/credentials/file_capabilities.rs b/kernel/src/process/credentials/file_capabilities.rs
> @@
> -        let value_len = match inode
> -            .get_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME, &mut value_writer)
> -        {
> +        let xattr_name =
> +            xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +        let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> 
> diff --git a/kernel/src/fs/fs_impls/ext2/impl_for_vfs/inode.rs b/kernel/src/fs/fs_impls/ext2/impl_for_vfs/inode.rs
> @@
>      fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
>          self.check_permission(Permission::MAY_READ)?;
>          self.get_xattr(name, value_writer)
>      }
> ```

`bug` (major): `execve()` now reads `security.capability` through `Inode::get_xattr`, but the filesystem implementations perform a normal DAC read check before looking up the xattr. A file can be executable without being readable, for example mode `0111`; in that case `FileCapabilities::read_from_inode` now returns `EACCES` before it can see `ENODATA`, so even an execute-only program with no file capabilities is rejected by `execve()`.

**Fix.** Shared with the other no-DAC xattr comments: restore narrow kernel-internal read/remove helpers for `security.capability` that bypass DAC permission checks while preserving filesystem/type validation, and use them from `FileCapabilities::read_from_inode` and `clear_file_capability_xattr`.

### `kernel/src/process/execve.rs` line 79

> ```diff
> diff --git a/kernel/src/process/execve.rs b/kernel/src/process/execve.rs
> @@
> +    let exec_euid = if elf_file.mode()?.has_set_uid() {
> +        elf_file.owner()?
> +    } else {
> +        credentials.euid()
> +    };
> +    let capsets_for_exec = credentials.prepare_capsets_for_exec(file_capabilities, exec_euid)?;
> @@
>      set_uid_from_elf(process, credentials, elf_inode)?;
>      set_gid_from_elf(process, credentials, elf_inode)?;
>      credentials.update_capsets_for_exec(capsets_for_exec);
> @@
>      if elf_inode.mode()?.has_set_uid() {
>          let uid = elf_inode.owner()?;
>          credentials.set_euid(uid);
> ```

`atomic-critical-sections` (critical): The capability sets are computed from a pre-exec snapshot of the file's setuid state, but `set_uid_from_elf` later re-reads the inode mode and owner before applying the already-computed caps. A concrete interleaving is: the file is setuid-root with no file-capability xattr at lines 74-79, so `prepare_capsets_for_exec` computes root effective/permitted capabilities; before line 333 another task clears the setuid bit or changes the owner; `set_uid_from_elf` observes the new metadata, but line 335 still installs the root-derived capsets into a non-root exec. The check and action are not using the same metadata snapshot.

**Fix.** Shared with the other exec metadata snapshot comment: make UID/GID changes and capability calculation consume one consistent executable metadata snapshot. For example, collect the mode/owner/group once into an `ExecIdentity`, use that same value both to set the effective IDs and to compute `ExecCapSets`, or move the non-fallible capability calculation back after `set_uid_from_elf` so it uses the credentials that were actually applied.

## Security

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
>     let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
>     match inode.remove_xattr(xattr_name) {
>         Ok(()) => Ok(()),
>         Err(error) if matches!(error.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => Ok(()),
>         Err(error) => Err(error),
>     }
> ```

`bug` (major): `clear_file_capability_xattr` is a kernel-internal privilege invalidation path, but it now calls `remove_xattr`, whose filesystem implementations perform DAC write checks against the current task. That can leave `security.capability` behind after a metadata-changing operation has already partially succeeded. For instance, `sys_fchown` changes uid/gid and then calls this helper; if the caller lacks DAC write permission on the resulting inode, this line returns `EACCES` after ownership changed, preserving file capabilities that should have been stripped.

**Fix.** Shared with the other no-DAC xattr comments: restore narrow kernel-internal read/remove helpers for `security.capability` that bypass DAC permission checks while preserving filesystem/type validation, and use them from `FileCapabilities::read_from_inode` and `clear_file_capability_xattr`; continue ignoring only `ENODATA`/`EOPNOTSUPP` in the clearing helper.

### `kernel/src/process/execve.rs` line 79

> ```diff
>     let exec_euid = if elf_file.mode()?.has_set_uid() {
>         elf_file.owner()?
>     } else {
>         credentials.euid()
>     };
>     let capsets_for_exec = credentials.prepare_capsets_for_exec(file_capabilities, exec_euid)?;
> ...
>     set_uid_from_elf(process, credentials, elf_inode)?;
>     set_gid_from_elf(process, credentials, elf_inode)?;
>     credentials.update_capsets_for_exec(capsets_for_exec);
> ```

`bug` (critical): `capsets_for_exec` is computed from `exec_euid` before `execve` becomes irreversible, but `apply_caps_from_exec` later re-reads the executable's setuid bit and owner when applying the UID. An attacker who owns the executable can race a setuid-bit change between these two points. For example, a process with real UID attacker and effective UID root execs an attacker-owned non-setuid file, so line 79 prepares root-capability sets; the attacker then sets the file's setuid bit before line 333, causing `set_uid_from_elf` to drop euid to the attacker while line 335 installs the previously computed root capability sets.

**Fix.** Shared with the other exec metadata snapshot comment: use one consistent snapshot for both the credential ID transition and capability calculation. For example, capture the executable mode/owner once and pass that snapshot to `set_uid_from_elf`, or move the non-fallible capability calculation to after `set_uid_from_elf` so it uses the actual post-setuid credentials; keep only the early validation that can return `EPERM` before the fatal point.
