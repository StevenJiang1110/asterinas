// SPDX-License-Identifier: MPL-2.0

use ostd::{cpu::context::UserContext, user::UserContextApi};

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::signal::{c_types::stack_t, SigStack, SigStackFlags, SIG_STACK_FLAGS_MASK},
};

pub fn sys_sigaltstack(
    sig_stack_addr: Vaddr,
    old_sig_stack_addr: Vaddr,
    ctx: &Context,
    user_ctx: &UserContext,
) -> Result<SyscallReturn> {
    debug!(
        "sig_stack_addr = 0x{:x}, old_sig_stack_addr: 0x{:x}",
        sig_stack_addr, old_sig_stack_addr
    );

    let sp = user_ctx.stack_pointer();

    get_old_stack(old_sig_stack_addr, ctx, sp)?;

    if sig_stack_addr != 0 {
        let stack = ctx.user_space().read_val::<stack_t>(sig_stack_addr)?;
        set_new_stack(stack, ctx, sp)?;
    };

    Ok(SyscallReturn::Return(0))
}

fn get_old_stack(old_sig_stack_addr: Vaddr, ctx: &Context, rsp: usize) -> Result<()> {
    if old_sig_stack_addr == 0 {
        return Ok(());
    }

    let old_stack = ctx.thread_local.sig_stack().borrow();

    let flags = old_stack.active_status(rsp) | old_stack.flags() & SIG_STACK_FLAGS_MASK;

    let stack = stack_t {
        ss_sp: old_stack.base(),
        ss_flags: flags.bits() as _,
        ss_size: old_stack.size(),
    };

    ctx.user_space()
        .write_val::<stack_t>(old_sig_stack_addr, &stack)
}

pub(super) fn set_new_stack(stack: stack_t, ctx: &Context, rsp: usize) -> Result<()> {
    let mut old_stack = ctx.thread_local.sig_stack().borrow_mut();

    if old_stack.contains(rsp) {
        return_errno_with_message!(Errno::EPERM, "the old stack is active now");
    }

    let flags = check_new_ss_flags(stack.ss_flags as u32)?;

    let new_stack = if flags.contains(SigStackFlags::SS_DISABLE) {
        SigStack::new(0, flags, 0)
    } else {
        if stack.ss_size < MINSTKSZ {
            return_errno_with_message!(Errno::ENOMEM, "stack size is less than MINSTKSZ");
        }

        if stack.ss_sp.checked_add(stack.ss_size).is_none() {
            return_errno_with_message!(Errno::EINVAL, "overflow for given stack addr and size");
        }

        SigStack::new(stack.ss_sp, flags, stack.ss_size)
    };

    debug!("new stack = {:x?}", new_stack);

    *old_stack = new_stack;

    Ok(())
}

fn check_new_ss_flags(ss_flags: u32) -> Result<SigStackFlags> {
    let ss_flags = SigStackFlags::from_bits(ss_flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown flags"))?;

    let flags = ss_flags & !SIG_STACK_FLAGS_MASK;
    if flags != SigStackFlags::SS_DISABLE
        && flags != SigStackFlags::SS_ONSTACK
        && flags != SigStackFlags::empty()
    {
        return_errno_with_message!(Errno::EINVAL, "invalid sig stack flags")
    }

    Ok(ss_flags)
}

#[expect(unused)]
const SIGSTKSZ: usize = 8192;
const MINSTKSZ: usize = 2048;
