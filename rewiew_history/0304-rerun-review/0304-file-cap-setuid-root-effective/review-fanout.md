---
date: 2026-07-03
mode: diff
base: 043ef13c6
head: f5fc357bb
branch: HEAD
---

# Summary

The change improves the shape of file-capability parsing by making revisions and flags explicit, and it tries to move `execve()` capability computation before the point where failures become fatal. The highest-risk problem is that this also removes the internal xattr accessors that bypass ordinary DAC checks. As a result, `execve()` can fail for execute-only files with `security.capability`, and capability-clearing cleanup can fail after an already-authorized write, truncate, or ownership change.

The other major issue is the new split between precomputed capability sets and later UID/GID application: both decisions need to use the same captured executable metadata, or the setuid bit and owner can change between the two reads. Structurally, restore a narrow kernel-internal file-capability xattr path, keep `security.capability` represented by a typed invariant instead of a raw string, and separate the real behavior changes from the parser refactor in commit history or messaging.

## Maintainability

### `commit f5fc357bb message`

> ```diff
> [commit message]
> Refactor file capability execve handling
> ```

`refactor-then-feature` (minor): The subject presents this as `Refactor file capability execve handling`, but the diff removes the `*_without_permission_check` inode API and changes `FileCapabilities::read_from_inode` and `clear_file_capability_xattr` to call `get_xattr` and `remove_xattr`. That hides a semantic permission-path change inside a refactor commit.

**Fix.** Split the permission-path change from the parser and `execve()` restructuring, or retitle the commit with a verb-first subject that describes the behavior change, such as `Fix file capability xattr access during execve`, with a body explaining the permission semantics.

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 11

> ```diff
> -pub(crate) const SECURITY_CAPABILITY_XATTR_NAME: XattrName<'static> =
> -    XattrName::from_known_valid_full_name(XattrNamespace::Security, "security.capability");
> +pub const SECURITY_CAPABILITY_XATTR_NAME: &str = "security.capability";
> ...
> +    let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> ```

`rust-type-invariants` (minor): `SECURITY_CAPABILITY_XATTR_NAME` used to be a validated `XattrName<'static>`, but it is now exported as a raw `&str`. Callers now have to rebuild the typed value with `XattrName::try_from_full_name(...).unwrap()`, so the namespace invariant lives in a string literal instead of the type system.

**Fix.** Restore a typed constant for kernel-owned fixed names, and compare with `SECURITY_CAPABILITY_XATTR_NAME.full_name()` where a string is needed. If necessary, keep a narrow `const fn` constructor for fixed internal names.

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 14

> ```diff
> /// Clears the file capability xattr after an operation invalidates file privileges.
> -pub(crate) fn clear_file_capability_xattr(inode: &dyn Inode) -> Result<()> {
> +pub fn clear_file_capability_xattr(inode: &dyn Inode) -> Result<()> {
> ```

`narrow-visibility` (minor): `clear_file_capability_xattr` is now `pub`, but it is only used by in-crate syscall and filesystem paths. Publishing it through `fs::vfs::xattr` widens a kernel-internal invalidation helper beyond its actual consumers.

**Fix.** Keep the visibility at `pub(crate)` unless an external crate actually needs this helper.

### `kernel/src/process/credentials/file_capabilities.rs` line 80

> ```diff
> VfsCapRevision::V1 => {
>     let Ok(permitted) = CapSet::try_from_lo_hi(read_u32_le(raw_value, 1)?, 0) else {
>         return_errno_with_message!(Errno::EINVAL, "file capabilities contain unsupported capability bits");
>     };
>     let Ok(inheritable) = CapSet::try_from_lo_hi(read_u32_le(raw_value, 2)?, 0) else {
>         return_errno_with_message!(Errno::EINVAL, "file capabilities contain unsupported capability bits");
>     };
>     (XATTR_CAPS_SZ_1, permitted, inheritable, None)
> }
> ```

`dry` (minor): `FileCapabilities::parse` repeats the same `CapSet::try_from_lo_hi` plus `Errno::EINVAL` translation for `permitted` and `inheritable` in every `VfsCapRevision` arm. The capability-word parsing and error text are one piece of knowledge, but this change spreads them across six blocks.

**Fix.** Factor the repeated conversion back into a helper, for example `read_capset(raw_value, lo_word_index, hi_word_index)`, and let each revision arm only describe its word layout.

## Correctness

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
> -    match inode.remove_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME) {
> +    let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +    match inode.remove_xattr(xattr_name) {
>          Ok(()) => Ok(()),
>          Err(error) if matches!(error.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => Ok(()),
>          Err(error) => Err(error),
> ```

Incorrect permission check (major): `clear_file_capability_xattr` now calls `inode.remove_xattr`, whose `ramfs` and `ext2` implementations enforce `check_permission(Permission::MAY_WRITE)`. This helper is used by `write`, `pwrite`, `truncate`, `ftruncate`, and `chown` after the operation itself has already authorized the mutation. A process can keep a writable file descriptor and later lose current DAC write permission; the subsequent `write` should still succeed through the descriptor and clear `security.capability`, but this path returns `EACCES` before the write.

**Fix.** Shared with the other xattr permission-bypass comments: restore narrowly scoped kernel-internal helpers for file-capability xattrs, so `FileCapabilities::read_from_inode` and `clear_file_capability_xattr` can read/remove `security.capability` without public DAC checks while still keeping normal user xattr syscalls permission-checked.

### `kernel/src/process/credentials/file_capabilities.rs` line 46

> ```diff
> -        let value_len = match inode
> -            .get_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME, &mut value_writer)
> -        {
> +        let xattr_name =
> +            xattr::XattrName::try_from_full_name(xattr::SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +        let value_len = match inode.get_xattr(xattr_name, &mut value_writer) {
> ```

Incorrect permission check (major): `FileCapabilities::read_from_inode` now calls `inode.get_xattr`, but the concrete `ramfs` and `ext2` `get_xattr` implementations enforce `check_permission(Permission::MAY_READ)`. That makes `execve` of an execute-only file with `security.capability` fail with `EACCES`: the caller can have execute permission on mode `0111`, yet cannot read the xattr through this public path. Reading file capabilities during `execve` is a kernel-internal operation and must not depend on the caller's DAC read permission.

**Fix.** Shared with the other xattr permission-bypass comments: restore narrowly scoped kernel-internal helpers for file-capability xattrs, so `FileCapabilities::read_from_inode` and `clear_file_capability_xattr` can read/remove `security.capability` without public DAC checks while still keeping normal user xattr syscalls permission-checked.

## Security

### `kernel/src/fs/vfs/fs_apis/xattr.rs` line 20

> ```diff
> -    match inode.remove_xattr_without_permission_check(SECURITY_CAPABILITY_XATTR_NAME) {
> +    let xattr_name = XattrName::try_from_full_name(SECURITY_CAPABILITY_XATTR_NAME).unwrap();
> +    match inode.remove_xattr(xattr_name) {
>          Ok(()) => Ok(()),
>          Err(error) if matches!(error.error(), Errno::ENODATA | Errno::EOPNOTSUPP) => Ok(()),
>          Err(error) => Err(error),
> ```

Privilege retention after partial cleanup (major): `clear_file_capability_xattr()` now calls the user-facing `inode.remove_xattr()`, which re-checks inode write permission. Callers such as `sys_fchown()` and `sys_fchownat()` mutate `path.set_owner()` / `path.set_group()` before calling this helper; after the ownership change, `remove_xattr()` can fail its `MAY_WRITE` check against the new metadata, returning an error after the metadata has already changed and leaving `security.capability` on the file.

**Fix.** Shared with the other xattr permission-bypass comments: restore narrowly scoped kernel-internal helpers for file-capability xattrs, so `FileCapabilities::read_from_inode` and `clear_file_capability_xattr` can read/remove `security.capability` without public DAC checks while still keeping normal user xattr syscalls permission-checked.

### `kernel/src/process/execve.rs` line 74

> ```diff
> +    let exec_euid = if elf_file.mode()?.has_set_uid() {
> +        elf_file.owner()?
> +    } else {
> +        credentials.euid()
> +    };
> +    let capsets_for_exec = credentials.prepare_capsets_for_exec(file_capabilities, exec_euid)?;
> ...
> +    set_uid_from_elf(process, credentials, elf_inode)?;
> +    set_gid_from_elf(process, credentials, elf_inode)?;
> +    credentials.update_capsets_for_exec(capsets_for_exec);
> ```

Time of check to time of use (major): `exec_euid` is computed from `elf_file.mode()` and `elf_file.owner()` before `start_execve()`, but `set_uid_from_elf()` later re-reads the inode metadata before `credentials.update_capsets_for_exec(capsets_for_exec)`. A concrete interleaving is: this code observes a setuid-root inode and precomputes root-derived capability sets, another task clears `S_ISUID` before line `333`, then `set_uid_from_elf()` leaves the caller's non-root `euid` unchanged while line `335` still installs the precomputed root capability sets.

**Fix.** Use one atomic/captured executable-credential decision for both UID changes and capability calculation. For example, capture the setuid decision and owner once, pass that through `do_execve_no_return()`, and make `set_uid_from_elf()` apply the same captured `exec_euid` that `prepare_capsets_for_exec()` used; alternatively, hold the inode metadata stable across both the check and use.
