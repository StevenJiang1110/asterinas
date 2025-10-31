// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use aster_rights::{ReadDupOp, ReadOp, WriteOp};
use ostd::sync::{RoArc, RwMutexReadGuard, Waker};

use super::{
    signal::{
        sig_mask::AtomicSigMask, sig_num::SigNum, sig_queues::SigQueues, signals::Signal,
        SigEvents, SigEventsFilter,
    },
    Credentials, Process,
};
use crate::{
    events::Observer,
    fs::{file_table::FileTable, thread_info::ThreadFsInfo},
    prelude::*,
    process::{namespace::nsproxy::NsProxy, Pid},
    thread::{Thread, Tid},
    time::{clocks::ProfClock, Timer, TimerManager},
};

mod builder;
mod exit;
pub mod futex;
mod name;
mod posix_thread_ext;
mod robust_list;
mod thread_local;
pub mod thread_table;

pub use builder::PosixThreadBuilder;
pub(super) use exit::sigkill_other_threads;
pub use exit::{do_exit, do_exit_group};
pub use name::{ThreadName, MAX_THREAD_NAME_LEN};
pub use posix_thread_ext::AsPosixThread;
pub use robust_list::RobustListHead;
pub use thread_local::{AsThreadLocal, FileTableRefMut, ThreadLocal};

pub struct PosixThread {
    // Immutable part
    process: Weak<Process>,

    // Mutable part
    tid: AtomicU32,

    name: Mutex<ThreadName>,

    /// Process credentials. At the kernel level, credentials are a per-thread attribute.
    credentials: Credentials,

    /// The file system information of the thread.
    fs: RwMutex<Arc<ThreadFsInfo>>,

    // Files
    /// File table
    file_table: Mutex<Option<RoArc<FileTable>>>,

    // Signal
    /// Blocked signals
    sig_mask: AtomicSigMask,
    /// Thread-directed sigqueue
    sig_queues: SigQueues,
    /// The per-thread signal [`Waker`], which will be used to wake up the thread
    /// when enqueuing a signal.
    signalled_waker: SpinLock<Option<Arc<Waker>>>,

    /// A profiling clock measures the user CPU time and kernel CPU time in the thread.
    prof_clock: Arc<ProfClock>,

    /// A manager that manages timers based on the user CPU time of the current thread.
    virtual_timer_manager: Arc<TimerManager>,

    /// A manager that manages timers based on the profiling clock of the current thread.
    prof_timer_manager: Arc<TimerManager>,

    /// I/O Scheduling priority value
    io_priority: AtomicU32,

    /// The namespaces that the thread belongs to.
    ns_proxy: Mutex<Option<Arc<NsProxy>>>,
}

impl PosixThread {
    pub fn process(&self) -> Arc<Process> {
        self.process.upgrade().unwrap()
    }

    pub fn weak_process(&self) -> Weak<Process> {
        Weak::clone(&self.process)
    }

    /// Returns the thread id
    pub fn tid(&self) -> Tid {
        self.tid.load(Ordering::Relaxed)
    }

    /// Sets the thread as the main thread by changing its thread ID.
    pub(super) fn set_main(&self, pid: Pid) {
        debug_assert_eq!(pid, self.process.upgrade().unwrap().pid());
        debug_assert_ne!(pid, self.tid.load(Ordering::Relaxed));

        self.tid.store(pid, Ordering::Relaxed);
    }

    pub fn thread_name(&self) -> &Mutex<ThreadName> {
        &self.name
    }

    /// Returns a read guard to the filesystem information of the thread.
    pub fn fs(&self) -> RwMutexReadGuard<Arc<ThreadFsInfo>> {
        self.fs.read()
    }

    /// Updates the filesystem information of the thread.
    pub fn update_fs(&self, new_fs: Arc<ThreadFsInfo>) {
        let mut fs_lock = self.fs.write();
        *fs_lock = new_fs;
    }

    pub fn file_table(&self) -> &Mutex<Option<RoArc<FileTable>>> {
        &self.file_table
    }

    /// Get the reference to the signal mask of the thread.
    ///
    /// Note that while this function offers mutable access to the signal mask,
    /// it is not sound for callers other than the current thread to modify the
    /// signal mask. They may only read the signal mask.
    pub fn sig_mask(&self) -> &AtomicSigMask {
        &self.sig_mask
    }

    pub(super) fn sig_queues(&self) -> &SigQueues {
        &self.sig_queues
    }

    /// Returns whether the signal is blocked by the thread.
    pub fn has_signal_blocked(&self, signum: SigNum) -> bool {
        // FIXME: Some signals cannot be blocked, even set in sig_mask.
        self.sig_mask.contains(signum, Ordering::Relaxed)
    }

    /// Sets the input [`Waker`] as the signalled waker of this thread.
    ///
    /// This approach can collaborate with signal-aware wait methods.
    /// Once a signalled waker is set for a thread, it cannot be reset until it is cleared.
    ///
    /// # Panics
    ///
    /// If setting a new waker before clearing the current thread's signalled waker
    /// this method will panic.
    pub fn set_signalled_waker(&self, waker: Arc<Waker>) {
        let mut signalled_waker = self.signalled_waker.lock();
        assert!(signalled_waker.is_none());
        *signalled_waker = Some(waker);
    }

    /// Clears the signalled waker of this thread.
    pub fn clear_signalled_waker(&self) {
        *self.signalled_waker.lock() = None;
    }

    /// Wakes up the signalled waker.
    pub fn wake_signalled_waker(&self) {
        if let Some(waker) = &*self.signalled_waker.lock() {
            waker.wake_up();
        }
    }

    /// Enqueues a thread-directed signal.
    ///
    /// This method does not perform permission checks on user signals.
    /// Therefore, unless the caller can ensure that there are no permission issues,
    /// this method should be used to enqueue kernel signals or fault signals.
    pub fn enqueue_signal(&self, signal: Box<dyn Signal>) {
        let process = self.process();
        let sig_dispositions = process.sig_dispositions().lock();
        let sig_dispositions = sig_dispositions.lock();

        let will_ignore = sig_dispositions.will_ignore(signal.as_ref());
        let blocked = self.has_signal_blocked(signal.num());
        if will_ignore && !blocked {
            return;
        }

        self.sig_queues.enqueue(signal);
        self.wake_signalled_waker();
    }

    /// Returns a reference to the profiling clock of the current thread.
    pub fn prof_clock(&self) -> &Arc<ProfClock> {
        &self.prof_clock
    }

    /// Creates a timer based on the profiling CPU clock of the current thread.
    pub fn create_prof_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.prof_timer_manager.create_timer(func)
    }

    /// Creates a timer based on the user CPU clock of the current thread.
    pub fn create_virtual_timer<F>(&self, func: F) -> Arc<Timer>
    where
        F: Fn() + Send + Sync + 'static,
    {
        self.virtual_timer_manager.create_timer(func)
    }

    /// Checks the `TimerCallback`s that are managed by the `prof_timer_manager`.
    /// If any have timed out, call the corresponding callback functions.
    pub fn process_expired_timers(&self) {
        self.prof_timer_manager.process_expired_timers();
    }

    pub fn register_sigqueue_observer(
        &self,
        observer: Weak<dyn Observer<SigEvents>>,
        filter: SigEventsFilter,
    ) {
        self.sig_queues.register_observer(observer, filter);
    }

    pub fn unregister_sigqueue_observer(&self, observer: &Weak<dyn Observer<SigEvents>>) {
        self.sig_queues.unregister_observer(observer);
    }

    /// Gets the read-only credentials of the thread.
    pub fn credentials(&self) -> Credentials<ReadOp> {
        self.credentials.dup().restrict()
    }

    /// Gets the duplicatable read-only credentials of the thread.
    pub fn credentials_dup(&self) -> Credentials<ReadDupOp> {
        self.credentials.dup().restrict()
    }

    /// Gets the write-only credentials of the current thread.
    ///
    /// It is illegal to mutate the credentials from a thread other than the
    /// current thread. For performance reasons, this function only checks it
    /// using debug assertions.
    pub fn credentials_mut(&self) -> Credentials<WriteOp> {
        debug_assert!(core::ptr::eq(
            current_thread!().as_posix_thread().unwrap(),
            self
        ));
        self.credentials.dup().restrict()
    }

    /// Returns the I/O priority value of the thread.
    pub fn io_priority(&self) -> &AtomicU32 {
        &self.io_priority
    }

    /// Returns the namespaces which the thread belongs to.
    pub fn ns_proxy(&self) -> &Mutex<Option<Arc<NsProxy>>> {
        &self.ns_proxy
    }
}

static POSIX_TID_ALLOCATOR: AtomicU32 = AtomicU32::new(1);

/// Allocates a new tid for the new posix thread
pub fn allocate_posix_tid() -> Tid {
    let tid = POSIX_TID_ALLOCATOR.fetch_add(1, Ordering::SeqCst);
    if tid >= PID_MAX {
        // When the kernel's next PID value reaches `PID_MAX`,
        // it should wrap back to a minimum PID value.
        // PIDs with a value of `PID_MAX` or larger should not be allocated.
        // Reference: <https://docs.kernel.org/admin-guide/sysctl/kernel.html#pid-max>.
        //
        // FIXME: Currently, we cannot determine which PID is recycled,
        // so we are unable to allocate smaller PIDs.
        warn!("the allocated ID is greater than the maximum allowed PID");
    }
    tid
}

/// Returns the last allocated tid
pub fn last_tid() -> Tid {
    POSIX_TID_ALLOCATOR.load(Ordering::SeqCst) - 1
}

/// The maximum allowed process ID.
//
// FIXME: The current value is chosen arbitrarily.
// This value can be modified by the user by writing to `/proc/sys/kernel/pid_max`.
pub const PID_MAX: u32 = u32::MAX / 2;
