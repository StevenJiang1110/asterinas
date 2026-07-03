---
date: 2026-07-03
mode: diff
base: 043ef13c6
head: f5fc357bb
branch: HEAD
---

# Summary

The refactor improves the shape of `execve()` capability handling by computing the new capability sets before the point where failure becomes fatal, and the new `ExecCapSets` type makes that handoff explicit.

The highest-risk issue is in the new `exec_euid` use for `file_effective`: a non-root caller executing a setuid-root file that also has a `security.capability` xattr can receive effective file capabilities even when the xattr's effective flag is unset. The other major regression is that internal file-capability xattr reads/removals now go through the user-facing DAC permission checks, which can break execute-only files and writes through already-open writable file descriptions.

Structurally, the new file-capability parser is clearer about revisions and flags, but the repeated `CapSet::try_from_lo_hi` conversion logic should be collapsed before it spreads further.

## Maintainability

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
> let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> match inode.remove_xattr(xattr_name) {
> ```

Incorrect permission check (major): `clear_file_capability_xattr` now calls `inode.remove_xattr`, but filesystem implementations such as `RamInode` and `Ext2Inode` check `MAY_WRITE` there. Callers like `InodeHandle::write` have already authorized the operation through the open file description, so a process can hold a writable fd, lose path write permission later, and then have `write()` fail only because this cleanup tries to remove `security.capability` through the user-facing permission path.

**Fix.** Shared with the corresponding correctness comment: restore a kernel-internal removal path for `security.capability` that skips DAC write-permission checks, while keeping user-triggered `removexattr()` on the normal permission-checked path.

### `kernel/src/process/credentials/file_capabilities.rs` line 47

> ```diff
> let xattr_name =
>     xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> ```

Incorrect permission check (major): `FileCapabilities::read_from_inode` now calls `inode.get_xattr`, but the concrete `get_xattr` implementations perform a DAC `MAY_READ` check. `execve()` is allowed for executable files that are not readable, so an executable with `security.capability` and mode like `--x--x--x` can now fail with `EACCES` while the kernel is only trying to read its own capability metadata.

**Fix.** Shared with the corresponding correctness comment: restore a kernel-internal read path for `security.capability` that bypasses the DAC read check, and use it from `FileCapabilities::read_from_inode()`.

### `kernel/src/process/credentials/file_capabilities.rs` line 79

> ```diff
> VfsCapRevision::V2 => {
>     let Ok(permitted) =
>         CapSet::try_from_lo_hi(read_u32_le(raw_value, 1)?, read_u32_le(raw_value, 3)?)
>     else {
>         return_errno_with_message!(
>             Errno::EINVAL,
>             "file capabilities contain unsupported capability bits"
>         );
>     };
> ```

`dry` (minor): The `VfsCapRevision` arms duplicate the same `CapSet::try_from_lo_hi` conversion and unsupported-bits error handling six times. Any future change to capability parsing or diagnostics has to be kept in sync across `V1`, `V2`, and `V3` by hand.

**Fix.** Extract a small helper such as `read_capset(raw_value, lo_word, hi_word)` and use it from each revision arm, leaving the match to describe only each revision's layout.

## Correctness

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
> -    match inode.remove_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME) {
> +    let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +    match inode.remove_xattr(xattr_name) {
> ```

Incorrect permission check (major): `clear_file_capability_xattr()` now calls `inode.remove_xattr()`, which performs a fresh `Permission::MAY_WRITE` check. That breaks valid writes through an already-open writable fd: open a file with `O_WRONLY`, then `chmod` it to remove write bits, and a later `write()` or `ftruncate()` should still succeed via the fd rights, but this helper returns `EACCES` before the write/truncate because the current inode mode is no longer writable.

**Fix.** Shared with the corresponding maintainability comment: restore an internal removal path for clearing `security.capability` that skips DAC write permission checks, and keep user-triggered `removexattr()` on the normal permission-checked path.

### `kernel/src/process/credentials/file_capabilities.rs` line 46

> ```diff
> -        let value_len = match inode
> -            .get_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME, &mut value_writer)
> -        {
> +        let xattr_name =
> +            xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +        let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> ```

Incorrect permission check (major): `FileCapabilities::read_from_inode()` now calls `inode.get_xattr()`, which ext2 and ramfs implement with a `Permission::MAY_READ` check. `execve()` only requires execute permission, so an executable such as mode `0111` now fails at the internal `security.capability` probe with `EACCES` even when the xattr is absent.

**Fix.** Shared with the corresponding maintainability comment: restore a kernel-internal xattr read path that bypasses the DAC read check for `security.capability`, and use it from `FileCapabilities::read_from_inode()`.

## Security

### `kernel/src/process/credentials/credentials_.rs` line 470

> ```diff
> @@
> -        let file_effective = if (!no_root && self.euid().is_root())
> +        let file_effective = if (!no_root && exec_euid.is_root())
>              || file_capabilities.is_some_and(FileCapabilities::has_effective_flag)
>          {
>              CapSet::all()
> ```

Incorrect capability escalation (critical): For a non-root caller executing a setuid-root file that also has a `security.capability` xattr with the effective flag unset, `exec_euid` is already root here, so `file_effective` becomes `CapSet::all()`. That makes `new_effective` include every newly permitted file capability at line `482`, even though the xattr deliberately did not request effective capabilities.

**Fix.** Keep the root effective shortcut for callers that were already root, and for setuid-root execution only when there is no file capability xattr. For example, distinguish the pre-exec `euid` from the computed `exec_euid`:

```rust
let file_effective = if (!no_root
    && (self.euid().is_root() || (!has_file_capabilities && exec_euid.is_root())))
    || file_capabilities.is_some_and(FileCapabilities::has_effective_flag)
{
    CapSet::all()
} else {
    CapSet::empty()
};
```
