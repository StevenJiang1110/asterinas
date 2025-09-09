// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, syscall::SyscallReturn};

pub fn sys_sethostname(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    let ns_context = ctx.thread_local.borrow_ns_context();
    let ns_context = ns_context.unwrap();
    let uts_ns = ns_context.uts_ns();
    let mut uts_name = uts_ns.uts_name().lock();
    if len > uts_name.nodename.len() - 1  {
        return_errno_with_message!(Errno::EINVAL, "the len is too long");
    }

    let nodename = uts_name.nodename.as_mut_slice();

    
    ctx.user_space().read_bytes(addr, &mut VmWriter::from(nodename))?;

    return Ok(SyscallReturn::Return(0));
}