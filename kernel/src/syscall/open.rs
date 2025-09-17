// SPDX-License-Identifier: MPL-2.0

use alloc::format;

use super::SyscallReturn;
use crate::{
    fs::{
        file_handle::FileLike,
        file_table::{FdFlags, FileDesc},
        fs_resolver::{FsPath, OpenArgs, AT_FDCWD},
        utils::{AccessMode, CreationFlags, StatusFlags},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_openat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    flags: u32,
    mode: u16,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, path = {:?}, flags = {}, mode = {}",
        dirfd, path, flags, mode
    );

    if path.is_empty() {
        return_errno_with_message!(Errno::ENOENT, "openat fails with empty path");
    }

    let open_args = OpenArgs::from_flags_and_mode(flags, mode)?;

    if path == CString::new("/proc/self/exe").unwrap() {
        // println!("open /proc/self/exe");

        let executable = ctx.process.executable.lock();
        if let Some(file) = executable.as_ref() {
            // println!("open executable of current processs");
            let filelike = file.clone();
            let fd = insert_file_like(ctx, filelike, flags, open_args);
            return Ok(SyscallReturn::Return(fd as _));
        }
    }

    let path_str = path.to_str().unwrap();
    if path_str.starts_with("/proc/self/fd/") {
        let fd = path_str.replace("/proc/self/fd/", "");
        let fd = fd.parse::<FileDesc>().unwrap();
        assert_eq!(format!("/proc/self/fd/{}", fd).as_str(), path_str);
        // println!("open: {}", path_str);

        let filelike = {
            let file_table = ctx.thread_local.borrow_file_table();
            let file_table_locked = file_table.unwrap().read();
            file_table_locked.get_file(fd)?.clone()
        };

        let new_fd = insert_file_like(ctx, filelike, flags, open_args);
        // println!("open {}, new_fd = {}", path_str, new_fd);
        return Ok(SyscallReturn::Return(new_fd as _));
    }

    let file_handle = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        let fs_ref = ctx.thread_local.borrow_fs();
        let mask_mode = mode & !fs_ref.umask().read().get();
        let inode_handle = fs_ref
            .resolver()
            .read()
            .open(&fs_path, flags, mask_mode)
            .map_err(|err| match err.error() {
                Errno::EINTR => Error::new(Errno::ERESTARTSYS),
                _ => err,
            })?;
        Arc::new(inode_handle)
    };

    let fd = insert_file_like(ctx, file_handle, flags, open_args);

    Ok(SyscallReturn::Return(fd as _))
}

fn open_named_pipe(filelike: &Arc<dyn FileLike>, open_args: OpenArgs) {
    let Ok(inode_handle) = filelike.as_inode_or_err() else {
        return;
    };

    let Some(inode) = inode_handle.inode() else {
        return;
    };

    let Some(named_pipe) = inode.as_fifo() else {
        return;
    };

    let abs_path = inode_handle.path().abs_path();
    debug!(
        "open named pipe: {}, access_mode = {:?}, status_flags = {:?}",
        abs_path, open_args.access_mode, open_args.status_flags
    );

    named_pipe.open(open_args);

    debug!("open named pipe successfully");
}

fn insert_file_like(
    ctx: &Context,
    filelike: Arc<dyn FileLike>,
    flags: u32,
    open_args: OpenArgs,
) -> FileDesc {
    open_named_pipe(&filelike, open_args);

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();
    let fd_flags = if CreationFlags::from_bits_truncate(flags).contains(CreationFlags::O_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };
    file_table_locked.insert(filelike, fd_flags)
}

pub fn sys_open(path_addr: Vaddr, flags: u32, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_openat(AT_FDCWD, path_addr, flags, mode, ctx)
}

pub fn sys_creat(path_addr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    let flags =
        AccessMode::O_WRONLY as u32 | CreationFlags::O_CREAT.bits() | CreationFlags::O_TRUNC.bits();
    self::sys_openat(AT_FDCWD, path_addr, flags, mode, ctx)
}
