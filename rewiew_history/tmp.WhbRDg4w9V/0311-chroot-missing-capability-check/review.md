---
date: 2026-07-03
mode: files
files: kernel/src/syscall/chroot.rs
head: 5b46e566d-dirty
branch: HEAD
---

# Summary

The implementation is small and follows the existing path-resolution flow, but it misses the authorization gate that makes `chroot(2)` a privileged operation. The main issue is that `sys_chroot()` updates the caller's root without checking `CAP_SYS_CHROOT`, so a process that has dropped that capability can still enter a new root.

Fix the capability check before `PathResolver::set_root()` and add a regression test that drops `CAP_SYS_CHROOT` and expects `EPERM`.

## Security

### `kernel/src/syscall/chroot.rs` line 28

> ```diff
> 14	    let fs_ref = ctx.thread_local.borrow_fs();
> 15	    let mut path_resolver = fs_ref.resolver().write();
> ...
> 25	    if path.type_() != InodeType::Dir {
> 26	        return_errno_with_message!(Errno::ENOTDIR, "must be directory");
> 27	    }
> 28	    path_resolver.set_root(path);
> ```

Missing privilege check (major): `sys_chroot()` accepts any caller that can resolve `path` and then calls `PathResolver::set_root()`. A process that has dropped `CAP_SYS_CHROOT` can still call `chroot("/tmp/jail")`, which violates Linux `chroot(2)` authorization and lets unprivileged code change its root view.

**Fix.** Check `CapSet::SYS_CHROOT` through `lsm_hooks::on_capable()` before installing the new root, using the caller's current user namespace, and add a regression test that drops the capability and expects `EPERM`.
