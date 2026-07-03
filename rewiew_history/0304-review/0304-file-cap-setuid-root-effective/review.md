---
date: 2026-07-02
mode: diff
base: 043ef13c6
head: f5fc357bb
branch: HEAD
---

# Summary

The refactor improves the shape of exec-time capability handling by separating validation from the no-return phase, but it also changes two security-sensitive boundaries.

Top issues:

- Critical: exec capability sets are prepared from one setuid/owner snapshot and later applied after re-reading inode metadata, so concurrent metadata changes can install capabilities that do not match the euid actually applied.
- Major: the file-capability effective-set calculation still treats setuid-root as an unconditional effective flag even when a `security.capability` xattr exists.
- Major: kernel-internal file-capability xattr reads/removals now go through public inode xattr methods, reintroducing DAC checks into exec and post-write cleanup paths that previously bypassed them.

Structurally, keep file capability xattr access behind a narrow internal helper, and make exec use one consistent executable metadata snapshot, or recompute capability sets only after the credential changes that will actually be committed.

## Correctness

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
>     let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
>     match inode.remove_xattr(xattr_name) {
>         Ok(()) => Ok(()),
> ```

`bug` (major): `clear_file_capability_xattr()` is a kernel-internal cleanup used after writes, truncates, and ownership changes, but it now calls `Inode::remove_xattr()`, which performs a fresh DAC write-permission check. A process can hold a valid writable fd, then lose DAC write permission because credentials or mode changed; a later write through that fd should still be authorized by the fd and should clear file capabilities, but this helper can return the DAC error before the write happens.

**Fix.** Restore a kernel-internal removal path for `security.capability` that bypasses inode DAC write checks while still treating `ENODATA` and `EOPNOTSUPP` as success.

### `kernel/src/process/credentials/file_capabilities.rs` line 46

> ```diff
>         let xattr_name =
>             xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
>         let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> ```

`bug` (major): Reading `security.capability` during exec now goes through `Inode::get_xattr()`, which performs a DAC read-permission check in ext2 and ramfs. Exec only requires execute permission on the file; an execute-only regular file on an xattr-capable filesystem will now fail with the read-permission error before the lookup can return `ENODATA`, even when it has no capability xattr.

**Fix.** Restore a kernel-internal xattr read path that bypasses DAC checks for file-capability loading, or add a narrowly scoped helper such as `get_file_capability_xattr()` implemented by filesystems without calling `check_permission(Permission::MAY_READ)`.

## Security

### `kernel/src/process/credentials/credentials_.rs` line 470

> ```diff
>         let file_effective = if (!no_root && exec_euid.is_root())
>             || file_capabilities.is_some_and(FileCapabilities::has_effective_flag)
>         {
>             CapSet::all()
>         } else {
>             CapSet::empty()
>         };
> ```

`bug` (major): The setuid-root file-capability exception is only applied to the permitted and inheritable file sets. `file_effective` still treats `exec_euid == root` as an unconditional effective flag even when a `security.capability` xattr exists. A non-root user executing a setuid-root binary with file permitted capabilities but without the effective flag will incorrectly receive those capabilities in the effective set.

**Fix.** Apply the same root-special-case predicate to the effective file set, e.g. base this branch on `grant_root_file_sets || file_capabilities.is_some_and(FileCapabilities::has_effective_flag)` instead of raw `exec_euid.is_root()`.

### `kernel/src/process/execve.rs` line 79

> ```diff
>     let exec_euid = if elf_file.mode()?.has_set_uid() {
>         elf_file.owner()?
>     } else {
>         credentials.euid()
>     };
>     let capsets_for_exec = credentials.prepare_capsets_for_exec(file_capabilities, exec_euid)?;
> ```

`bug` (critical): The prepared capability sets are computed from a snapshot of the executable's setuid bit and owner, but `apply_caps_from_exec()` later re-reads the inode metadata when setting the euid. A concurrent chmod/chown can make these disagree: for example, an unprivileged task starts exec of a setuid-root executable with no file capabilities, line 74 computes `exec_euid == root` and line 79 prepares full root capabilities; before line 333 runs, another task clears the setuid bit, so `set_uid_from_elf()` leaves the euid unprivileged, but line 335 still installs the precomputed root capability sets.

**Fix.** Use one consistent metadata snapshot for both the setuid/setgid credential changes and the capability calculation, or recompute the final capability sets after applying the actual uid/gid changes. The pre-irreversible phase may validate the file-capability EPERM condition, but the no-return phase must not apply capsets derived from metadata that is re-read differently later.
