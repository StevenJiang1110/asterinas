---
date: 2026-07-02
mode: diff
base: f374891ea
head: 2ae030b7f
branch: HEAD
---

# Summary

The refactor improves the structure around file capability parsing and moves more of the
`execve()` capability work before the no-return point. The main remaining issue is that
`execve()` now computes capability sets from one read of executable metadata, then later
re-reads the inode to apply setuid/setgid changes; those two reads can disagree. There are
also two minor maintainability regressions in the file-capability parser API and error text.

## Maintainability

### `kernel/src/process/credentials/file_capabilities.rs` line 80

> ```diff
> pub(in crate::process) const fn root_uid(&self) -> Option<Uid> {
>     self.user_ns_owner_uid_in_init_user_ns
> }
> ```

`information-hiding` (minor): `root_uid()` exposes the xattr encoding detail that `None` means a V1/V2 capability bound to UID 0 in the initial user namespace, forcing `execve` to reimplement that policy at the call site. A future caller can easily treat `None` as "applies everywhere" or "has no root UID" and get the capability applicability rule wrong.

**Fix.** Keep the policy inside `FileCapabilities`, for example restore a predicate like `applies_to_root_uid(self, root_uid: Uid) -> bool` (or rename it to `applies_to_user_ns_owner_uid`) and have `execve` call that instead of inspecting the `Option`.

### `kernel/src/process/credentials/file_capabilities.rs` line 121

> ```diff
> fn parse_revision_and_flags(magic_etc: u32) -> Result<(VfsCapRevision, VfsCapFlags)> {
>     let revision_bits = magic_etc & VFS_CAP_REVISION_MASK;
>     let revision = VfsCapRevision::try_from(revision_bits)?;
> ```

`error-message-format` (minor): Unsupported file-capability revisions now propagate the generic `TryFromIntError` message (`"Invalid enum value"`), which is uppercase and loses the syscall-specific context the old code provided. A malformed `security.capability` xattr with unknown revision bits triggers this path.

**Fix.** Map the enum conversion error locally to a specific lowercase message, e.g. `let revision = VfsCapRevision::try_from(revision_bits).map_err(|_| Error::with_message(Errno::EINVAL, "file capabilities use an unsupported xattr revision"))?;`.

## Correctness

### `kernel/src/process/execve.rs` line 80

> ```diff
> @@
> -    let exec_euid = if elf_file.mode()?.has_set_uid() {
> -        elf_file.owner()?
> +    let exec_euid = if elf_file.mode()?.has_set_uid() {
> +        elf_file.owner()?
> @@
> -    set_uid_from_elf(process, credentials, elf_inode)?;
> +    set_uid_from_elf(process, credentials, elf_inode)?;
> ```

`atomic-critical-sections` (major): The setuid decision used to compute `exec_euid` is read before the no-return phase, but `set_uid_from_elf()` re-reads the inode mode/owner later at lines 360-362. If the executable metadata changes in between, capability calculation and UID application can use different file states. For example, a file is setuid-root when line 80 runs so the cached capsets grant root capabilities; another task then clears the setuid bit or changes the owner before line 360; `set_uid_from_elf()` no longer sets euid to root, but line 345 still installs the root-capability capsets.

**Fix.** Use one metadata snapshot for both decisions, or revalidate after the later metadata read before applying cached capsets. A practical fix is to read mode/owner once into an exec metadata struct before validation and pass that same struct through to `set_uid_from_elf`/`set_gid_from_elf`, or recompute capsets from the exact mode/owner read in `apply_caps_from_exec` and fail before the no-return point if the metadata changed.

## Retracted by verification

- `kernel/src/process/execve.rs` line 85, Correctness: Refuted because sibling POSIX threads receive `Credentials::new_from`, which creates a new `Credentials_` snapshot instead of sharing the current thread's credential atomics.
- `kernel/src/process/execve.rs` line 85, Security: Refuted for the same reason; another thread in the same process cannot change this thread's bounding set or securebits through the shared-credentials race described by the comment.
