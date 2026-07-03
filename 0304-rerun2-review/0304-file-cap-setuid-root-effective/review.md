---
date: 2026-07-03
mode: diff
base: 043ef13c6
head: f5fc357bb
branch: HEAD
title: "Review of file capability execve refactor"
---

# Summary

The refactor improves the shape of file capability handling by parsing the xattr into a typed representation and computing exec capability sets before the irreversible part of `execve()`. The main problems are in preserving kernel-internal privilege semantics: file capability reads and clears now flow through user-facing xattr permission checks, and the setuid-root-plus-file-capability case grants effective capabilities too broadly.

Top issues:

1. `security.capability` clearing can fail for already-authorized writes/truncates because `clear_file_capability_xattr()` rechecks current DAC write permission.
2. `execve()` can fail to read file capabilities from execute-only files because `FileCapabilities::read_from_inode()` now uses permission-checked `get_xattr()`.
3. Setuid-root binaries with file capabilities but no effective flag can receive effective capabilities immediately, contrary to Linux's file-capability transformation rules.

## Correctness

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
>     let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
>     match inode.remove_xattr(xattr_name) {
>         Ok(()) => Ok(()),
> ```

Incorrect permission check (major): `clear_file_capability_xattr()` now calls `inode.remove_xattr()`, which rechecks DAC write permission. This breaks already-authorized operations such as `write()` or `ftruncate()` through an open writable fd after mode/ownership changes: the fd still has `Rights::WRITE`, but clearing `security.capability` can fail with `EACCES` before the write/truncate proceeds.

**Fix.** Shared with the other internal xattr-access comment: restore kernel-internal read/remove helpers for `security.capability` that bypass DAC checks, use them only from kernel file-capability maintenance paths, and keep the normal permission-checked xattr methods for user syscalls.

### `kernel/src/process/credentials/file_capabilities.rs` line 46

> ```diff
>         let xattr_name =
>             xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
>         let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> ```

Incorrect permission check (major): `read_from_inode()` now calls `inode.get_xattr()`, but the concrete `get_xattr()` implementations perform DAC read checks. A user can execute an execute-only file with `security.capability`, and `execve()` must still be able to read that xattr internally; with this path, `ramfs`/`ext2` return `EACCES` before `execve()` can apply the file capabilities.

**Fix.** Shared with the other internal xattr-access comment: restore kernel-internal read/remove helpers for `security.capability` that bypass DAC checks, use them only from kernel file-capability maintenance paths, and keep the normal permission-checked xattr methods for user syscalls.

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

Privilege escalation (major): `file_effective` is forced to `CapSet::all()` whenever the post-`execve()` EUID is root, even when the executable has a `security.capability` xattr. For a non-root caller executing a setuid-root binary with file permitted caps but without the file effective flag, this grants those caps in `effective` immediately; Linux's setuid-root-plus-file-capability exception honors the file effective bit in that case.

**Fix.** Do not use the root fast path for `file_effective` when `has_file_capabilities` is true and `exec_euid` became root only because of setuid. Preserve the file effective flag for that case, matching the same exception already applied to `file_permitted`/`file_inheritable`.
