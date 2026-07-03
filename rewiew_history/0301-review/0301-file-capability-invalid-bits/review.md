---
date: 2026-07-02
mode: diff
base: f374891ea
head: 2ae030b7f
branch: HEAD
---

# Summary

The refactor makes capability computation explicit before the irreversible part of
`execve()`, which is the right direction for avoiding late failures. The remaining
issue is that the code now computes capabilities from one observation of executable
metadata and applies setuid/setgid effects from a later observation.

Top issue: a major security bug in `execve()` can install full root capability sets
even if the executable no longer applies a root setuid transition by the time
credentials are updated. Use a single prepared metadata snapshot for both
capability calculation and credential application.

## Security

### `kernel/src/process/execve.rs` line 80

> ```diff
> let exec_euid = if elf_file.mode()?.has_set_uid() {
>     elf_file.owner()?
> } else {
>     credentials.euid()
> };
> let capsets_for_exec = credentials.prepare_capsets_for_exec(file_capabilities, exec_euid)?;
> ...
> if elf_inode.mode()?.has_set_uid() {
>     let uid = elf_inode.owner()?;
>     credentials.set_euid(uid);
> }
> ...
> credentials.update_capsets_for_exec(capsets_for_exec);
> ```

`bug` (major): `exec_euid` is sampled before the irreversible exec phase, but `set_uid_from_elf()` later re-reads the inode mode and owner when applying credentials. If line 80 sees a setuid-root executable and computes full root capability sets, then the setuid bit or owner changes before line 360, the later UID update can leave the task non-root while line 345 still installs the previously computed full permitted/effective capabilities.

**Fix.** Use one prepared metadata snapshot for both capability computation and credential application. For example, capture `setuid_uid: Option<Uid>` and `setgid_gid: Option<Gid>` before `prepare_capsets_for_exec()`, pass that prepared exec-credential data into `do_execve_no_return()`, and have `set_uid_from_elf()`/`set_gid_from_elf()` apply those values instead of re-reading `mode()` and `owner()`/`group()` after capabilities have been computed.
