// SPDX-License-Identifier: MPL-2.0

//! This module defines struct `ProcessVm`
//! to represent the layout of user space process virtual memory.
//!
//! The `ProcessVm` struct contains `Vmar`,
//! which stores all existing memory mappings.
//! The `Vm` also contains
//! the basic info of process level vm segments,
//! like init stack and heap.

mod heap;
mod init_stack;

#[cfg(target_arch = "riscv64")]
use core::sync::atomic::{AtomicUsize, Ordering};

use aster_rights::Full;
pub use heap::Heap;

pub use self::{
    heap::USER_HEAP_SIZE_LIMIT,
    init_stack::{
        aux_vec::{AuxKey, AuxVec},
        InitStack, InitStackReader, INIT_STACK_SIZE, MAX_LEN_STRING_ARG, MAX_NR_STRING_ARGS,
    },
};
use crate::{prelude::*, vm::vmar::Vmar};

/*
 * The user's virtual memory space layout looks like below.
 * TODO: The layout of the userheap does not match the current implementation,
 * And currently the initial program break is a fixed value.
 *
 *  (high address)
 *  +---------------------+ <------+ The top of Vmar, which is the highest address usable
 *  |                     |          Randomly padded pages
 *  +---------------------+ <------+ The base of the initial user stack
 *  | User stack          |
 *  |                     |
 *  +---------||----------+ <------+ The user stack limit, can be extended lower
 *  |         \/          |
 *  | ...                 |
 *  |                     |
 *  | MMAP Spaces         |
 *  |                     |
 *  | ...                 |
 *  |         /\          |
 *  +---------||----------+ <------+ The current program break
 *  | User heap           |
 *  |                     |
 *  +---------------------+ <------+ The original program break
 *  |                     |          Randomly padded pages
 *  +---------------------+ <------+ The end of the program's last segment
 *  |                     |
 *  | Loaded segments     |
 *  | .text, .data, .bss  |
 *  | , etc.              |
 *  |                     |
 *  +---------------------+ <------+ The bottom of Vmar at 0x1_0000
 *  |                     |          64 KiB unusable space
 *  +---------------------+
 *  (low address)
 */

/// The process user space virtual memory.
pub struct ProcessVm {
    vmar: Option<Vmar<Full>>,
    inner: Arc<ProcessVmInner>,
}

impl Clone for ProcessVm {
    fn clone(&self) -> Self {
        Self {
            vmar: self.vmar.as_ref().map(|vmar| vmar.dup().unwrap()),
            inner: self.inner.clone(),
        }
    }
}

struct ProcessVmInner {
    heap: Heap,
    init_stack: InitStack,
    #[cfg(target_arch = "riscv64")]
    vdso_base: AtomicUsize,
}

impl ProcessVm {
    /// Allocates a new `ProcessVm`.
    pub fn alloc() -> Self {
        let vmar = Vmar::<Full>::new_root();
        let init_stack = InitStack::new();
        let heap = Heap::new();
        heap.alloc_and_map_vm(&vmar).unwrap();
        let inner = ProcessVmInner {
            heap,
            init_stack,
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(0),
        };

        Self {
            vmar: Some(vmar),
            inner: Arc::new(inner),
        }
    }

    pub fn vmar(&self) -> Option<&Vmar<Full>> {
        self.vmar.as_ref()
    }

    /// Sets a new VMAR for the binding process.
    ///
    /// If the `new_vmar` is `None`, this method will remove the
    /// current VMAR.
    pub(super) fn set_vmar(&mut self, new_vmar: Option<Vmar<Full>>) {
        self.vmar = new_vmar;
    }

    /// Forks a `ProcessVm` from `other`.
    ///
    /// The returned `ProcessVm` will have a forked `Vmar`.
    pub fn fork_from(other: &ProcessVm) -> Result<Self> {
        let process_vmar = other.vmar.as_ref().unwrap();
        let vmar = Some(Vmar::<Full>::fork_from(process_vmar)?);

        let inner = ProcessVmInner {
            heap: other.inner.heap.clone(),
            init_stack: other.inner.init_stack.clone(),
            #[cfg(target_arch = "riscv64")]
            vdso_base: AtomicUsize::new(other.inner.vdso_base.load(Ordering::Relaxed)),
        };
        Ok(Self {
            vmar,
            inner: Arc::new(inner),
        })
    }

    /// Returns a reader for reading contents from
    /// the `InitStack`.
    pub fn init_stack_reader(&self) -> InitStackReader {
        self.inner.init_stack.reader(self.vmar().unwrap())
    }

    /// Returns the top address of the user stack.
    pub fn user_stack_top(&self) -> Vaddr {
        self.inner.init_stack.user_stack_top()
    }

    pub(super) fn map_and_write_init_stack(
        &self,
        argv: Vec<CString>,
        envp: Vec<CString>,
        aux_vec: AuxVec,
    ) -> Result<()> {
        let vmar = self.vmar.as_ref().unwrap();
        self.inner
            .init_stack
            .map_and_write(vmar, argv, envp, aux_vec)
    }

    pub fn heap(&self) -> &Heap {
        &self.inner.heap
    }

    #[cfg(target_arch = "riscv64")]
    pub(super) fn vdso_base(&self) -> Vaddr {
        self.inner.vdso_base.load(Ordering::Relaxed)
    }

    #[cfg(target_arch = "riscv64")]
    pub(super) fn set_vdso_base(&self, addr: Vaddr) {
        self.inner.vdso_base.store(addr, Ordering::Relaxed);
    }

    /// Clears existing mappings and then maps the heap VMO to the current VMAR.
    pub(super) fn clear_and_map_heap(&self) {
        let vmar = self.vmar().unwrap();
        vmar.clear().unwrap();
        self.inner.heap.alloc_and_map_vm(vmar).unwrap();
    }
}

/// Unshares and renews the [`ProcessVm`] of of the current process.
pub(super) fn unshare_and_renew_vm(ctx: &Context) {
    let mut process_vm = ctx.process.vm().lock();

    let new_vmar = Vmar::<Full>::new_root();
    *ctx.thread_local.root_vmar().borrow_mut() = Some(new_vmar.dup().unwrap());
    new_vmar.vm_space().activate();
    process_vm.set_vmar(Some(new_vmar));

    process_vm
        .inner
        .heap
        .alloc_and_map_vm(process_vm.vmar().unwrap())
        .unwrap();
}
