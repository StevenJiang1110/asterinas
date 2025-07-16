// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, process::signal::c_types::stack_t};

/// User-provided signal stack. `SigStack` is per-thread, and each thread can have
/// at most one `SigStack`. If one signal handler specifying the `SA_ONSTACK` flag,
/// the handler should be executed on the `SigStack`, instead of on the default stack.
///
/// SigStack can be registered and unregistered by syscall `sigaltstack`.
#[derive(Debug, Clone, Copy)]
pub struct SigStack {
    base: Vaddr,
    flags: SigStackFlags,
    size: usize,
}

impl Default for SigStack {
    fn default() -> Self {
        Self {
            base: Default::default(),
            flags: SigStackFlags::empty(),
            size: Default::default(),
        }
    }
}

bitflags! {
    pub struct SigStackFlags: u32 {
        const SS_ONSTACK = 1 << 0;
        const SS_DISABLE = 1 << 1;
        const SS_AUTODISARM = 1 << 31;
    }
}

pub const SIG_STACK_FLAGS_MASK: SigStackFlags = SigStackFlags::SS_AUTODISARM;

impl SigStack {
    pub fn new(base: Vaddr, flags: SigStackFlags, size: usize) -> Self {
        Self { base, flags, size }
    }

    pub fn base(&self) -> Vaddr {
        self.base
    }

    pub fn flags(&self) -> SigStackFlags {
        self.flags
    }

    pub fn active_status(&self, sp: usize) -> SigStackFlags {
        if self.size == 0 {
            return SigStackFlags::SS_DISABLE;
        }

        if self.contains(sp) {
            return SigStackFlags::SS_ONSTACK;
        }

        SigStackFlags::empty()
    }

    pub fn size(&self) -> usize {
        self.size
    }

    /// Returns whether the `sp` is on signal stack.
    pub fn contains(&self, sp: usize) -> bool {
        if self.flags().contains(SigStackFlags::SS_AUTODISARM) {
            return false;
        }

        // The stack grows down, so self.base is exclusive.
        sp > self.base && sp <= self.base + self.size
    }

    pub fn reset(&mut self) {
        self.base = 0;
        self.size = 0;
        self.flags = SigStackFlags::SS_DISABLE;
    }
}

impl From<&SigStack> for stack_t {
    fn from(value: &SigStack) -> Self {
        Self {
            ss_sp: value.base,
            ss_flags: value.flags.bits as _,
            ss_size: value.size,
        }
    }
}
