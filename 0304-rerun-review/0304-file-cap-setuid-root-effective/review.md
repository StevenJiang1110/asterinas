---
date: 2026-07-03
mode: diff
base: 043ef13c6
head: f5fc357bb
branch: HEAD
---

# Summary

This change improves the shape of exec capability handling by computing an `ExecCapSets` value before entering the no-return exec path, and the capability xattr parser is more strongly typed after the revision/flag refactor.

The main risk is security-sensitive permission handling around `security.capability`. Two findings share the same root cause: file capability operations that must be kernel-internal now go through DAC-checked xattr methods. That can break execute-only file-capability execs and can leave file capabilities behind after privilege-invalidating metadata changes. The highest-severity issue is the new `exec_euid` snapshot: capability computation and the later setuid application can observe different inode metadata, which can grant root-derived capabilities to a non-root exec.

## Correctness

### `kernel/src/process/credentials/file_capabilities.rs` line 46

> ```diff
> -        let value_len = match inode
> -            .get_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME, &mut value_writer)
> -        {
> +        let xattr_name =
> +            xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +        let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> ```

Incorrect permission check (major): `read_from_inode()` now calls `inode.get_xattr()`, but the ext2 and ramfs implementations check `Permission::MAY_READ` before looking up the xattr. A user can execute a mode `0111` file without read permission, so this makes `execve()` fail with `EACCES` while merely probing `security.capability`; the existing `file_caps_execute_only` regression is exactly this case.

**Fix.** Shared with the other xattr-permission comment: restore a kernel-internal `security.capability` access path that bypasses DAC checks for exec-time reads and privilege-invalidation cleanup, or expose narrower helpers dedicated to file-capability loading/clearing and use them here instead of `Inode::get_xattr()`.

## Security

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
> -    match inode.remove_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME) {
> +    let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +    match inode.remove_xattr(xattr_name) {
>          Ok(()) => Ok(()),
> ```

Incorrect cleanup (major): `clear_file_capability_xattr()` now calls `inode.remove_xattr()`, whose filesystem implementations perform the current task's DAC `MAY_WRITE` check. This helper is used after privilege-invalidating operations such as `chown()`/`fchown()`; if ownership is changed first and the post-change inode no longer grants write permission to the caller, `remove_xattr()` can fail after the owner was already changed, leaving `security.capability` in place.

**Fix.** Shared with the other xattr-permission comment: restore a kernel-internal `security.capability` access path that bypasses DAC checks for exec-time reads and privilege-invalidation cleanup, or move the invalidating metadata changes into inode methods that atomically update metadata and remove `security.capability` without rechecking user write permission.

### `kernel/src/process/execve.rs` line 74

> ```diff
> +    let exec_euid = if elf_file.mode()?.has_set_uid() {
> +        elf_file.owner()?
> +    } else {
> +        credentials.euid()
> +    };
> +    let capsets_for_exec = credentials.prepare_capsets_for_exec(file_capabilities, exec_euid)?;
>  
>      // Ensure no other thread is concurrently performing exit_group or execve.
> ```

Time-of-check/time-of-use (critical): `exec_euid` is snapshotted before `task_set.start_execve()`, but `set_uid_from_elf()` later re-reads `elf_inode.mode()` and `elf_inode.owner()`. If another task changes the executable from setuid-root to non-setuid, or changes its owner, between these two reads, `prepare_capsets_for_exec()` can compute root-derived `capsets_for_exec` while `set_uid_from_elf()` no longer sets `euid` to root, and `update_capsets_for_exec()` then grants full capabilities to a non-root exec.

**Fix.** Compute the UID transition and capability sets from one consistent inode state at the point where they are applied. For example, after `task_set.start_execve()` and immediately before `update_capsets_for_exec()`, read `mode`/`owner` once, use that same snapshot for both `set_uid_from_elf()` and `prepare_capsets_for_exec()`, and keep the existing pre-fatal validation separate if needed.
