---
date: 2026-07-02
mode: files
files: kernel/src/process/credentials/credentials_.rs,kernel/src/process/credentials/file_capabilities.rs
head: f5fc357bb-dirty
branch: HEAD
title: "Asterinas credential capability review"
---

# Summary

The credential code is compact and keeps the capability transformation logic in one place, which made the file-capability and setuid paths traceable. I found two major capability-state bugs:

1. `set_fsuid` drops filesystem-related effective capabilities when leaving fsuid root, but does not restore the permitted subset when returning to fsuid root.
2. `execve` applies the setuid-root/file-capability exception to the permitted and inheritable file sets, but not to the effective path, so mixed setuid-root binaries with file capabilities can make file capabilities effective when Linux would not.

Both findings were checked against the local call sites and Linux `security/commoncap.c`; no xattr parsing issue was kept after verifying the in-tree xattr implementations return `ERANGE` for too-small read buffers.

## Correctness

### `kernel/src/process/credentials/credentials_.rs` line 306

> ```diff
> 302     fn set_fsuid_unchecked(&self, fsuid: Uid) {
> 303         let old_fsuid = self.fsuid();
> 304         self.fsuid.store(fsuid, Ordering::Relaxed);
> 305 
> 306         if old_fsuid.is_root() && !fsuid.is_root() {
> 307             // Reference: The "Effect of user ID changes on capabilities" section in
> 308             // <https://man7.org/linux/man-pages/man7/capabilities.7.html>.
> 309             let cap_to_remove = CapSet::CHOWN
> 310                 | CapSet::DAC_OVERRIDE
> ...
> 317             let old_cap = self.effective_capset();
> 318             self.set_effective_capset(old_cap - cap_to_remove);
> 319         }
> 320     }
> ```

`bug` (major): Dropping from fsuid 0 to a nonzero fsuid removes the filesystem-related effective capabilities, but the reverse transition never restores the subset from the permitted set. A root task can call `setfsuid(65534)` and then `setfsuid(0)` with `CAP_CHOWN`, `CAP_DAC_OVERRIDE`, etc. still permitted, yet those capabilities remain absent from `effective_capset` until some unrelated euid transition re-enables them. Linux raises that filesystem capability subset when fsuid becomes root again, so this breaks reversible credential changes and permission checks that use effective capabilities.

**Fix.** Mirror the 0 -> nonzero branch with a nonzero -> 0 branch: factor the filesystem capability subset into a helper, drop it when leaving fsuid root, and when entering fsuid root set `effective_capset` to `old_cap | (permitted_capset & filesystem_capset)`.

## Security

### `kernel/src/process/credentials/credentials_.rs` line 470

> ```diff
> 453         // Linux treats root specially when the executable has no file capabilities, or when the
> 454         // real UID is root. The setuid-root + file-capability exception is handled by excluding
> 455         // the `euid == 0` fast path when a file capability xattr is present.
> 456         let grant_root_file_sets =
> 457             !no_root && (self.ruid().is_root() || (!has_file_capabilities && exec_euid.is_root()));
> ...
> 470         let file_effective = if (!no_root && exec_euid.is_root())
> 471             || file_capabilities.is_some_and(FileCapabilities::has_effective_flag)
> 472         {
> 473             CapSet::all()
> 474         } else {
> 475             CapSet::empty()
> 476         };
> ```

`bug` (major): The setuid-root plus file-capability exception is only applied to `file_permitted`/`file_inheritable`; `file_effective` still treats any `exec_euid == 0` as `CapSet::all()`. If a non-root process executes a setuid-root binary that also has a `security.capability` xattr without the effective flag, `grant_root_file_sets` limits the new permitted set to the file capability, but this branch marks the file effective set as all capabilities. The result is that every newly permitted file capability becomes effective, even though Linux deliberately does not raise all root effective privileges in the mixed setuid-root/file-capability case.

**Fix.** Use the same mixed-case predicate for the effective path. For example, derive an `effective_due_to_root` boolean from the root-special handling only when the executable should receive root file sets, and otherwise let only `FileCapabilities::has_effective_flag` make `new_permitted` effective.
