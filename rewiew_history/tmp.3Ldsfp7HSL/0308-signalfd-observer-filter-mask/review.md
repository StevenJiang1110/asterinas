---
date: 2026-07-02
mode: files
files: kernel/src/syscall/signalfd.rs,kernel/src/process/signal/events.rs,kernel/src/process/signal/sig_queues.rs
head: 20c2e967f
branch: HEAD
---

# Summary

The reviewed code is small and follows the existing signal subsystem shape, but the confirmed issues are all correctness bugs in observable signal behavior. The highest-priority problem is the inverted `SigEventsFilter`, which prevents signalfd poll waiters from being notified for the signals they registered to receive. The other two issues are also user-visible: blocked real-time signals are treated as deliverable by `has_pending`, and updating an existing signalfd incorrectly changes its nonblocking status.

Structurally, the fixes should keep the existing design: correct the event-filter predicate, make real-time pending checks honor the supplied blocked mask by deriving the signal number from the queue index, and leave file status flags untouched when `signalfd4` updates an existing descriptor.

## Correctness

### `kernel/src/process/signal/events.rs` line 35

> ```diff
> impl EventsFilter<SigEvents> for SigEventsFilter {
>     fn filter(&self, event: &SigEvents) -> bool {
>         !self.0.contains(event.0)
>     }
> }
> ```

`bug` (major): The signal-event filter is inverted. `Subject::notify_observers` calls `on_events` only when `filter.filter(events)` returns true, so a signalfd registered for SIGUSR1 is not notified for SIGUSR1; it is only called for signals outside the mask, which `SignalFile::on_events` then ignores. A blocking `poll` can sleep forever if the signal arrives after the poller registers.

**Fix.** Make the filter return true for signals included in the registered mask, e.g. `self.0.contains(event.sig_num())`.

### `kernel/src/process/signal/sig_queues.rs` line 207

> ```diff
> }) || self.rt_queues.iter().any(|rt_queue| !rt_queue.is_empty())
> ```

`bug` (major): `Queues::has_pending` ignores the blocked mask for real-time signals. If the only pending signal is a blocked real-time signal, `PosixThread::has_pending` still returns true, so waits can be interrupted with EINTR and user-mode execution can repeatedly exit for a signal that `dequeue` will not deliver.

**Fix.** Check the real-time signal number against `blocked` before treating a non-empty queue as deliverable.

### `kernel/src/syscall/signalfd.rs` line 117

> ```diff
> if signal_file.mask().load(Ordering::Relaxed) != new_mask {
>     signal_file.update_signal_mask(new_mask)?;
> }
> signal_file.set_non_blocking(non_blocking);
> ```

`bug` (major): Updating an existing signalfd incorrectly rewrites the file status from the `signalfd4` flags. On Linux, `SFD_NONBLOCK`/`SFD_CLOEXEC` affect only a newly created descriptor; `signalfd4(existing_fd, ..., 0)` must not clear an existing `O_NONBLOCK`, and `signalfd4(existing_fd, ..., SFD_NONBLOCK)` must not set it.

**Fix.** Do not pass `non_blocking` into `update_existing_signalfd`; when `fd != -1`, only replace the signal mask and leave status flags unchanged.

## Retracted by verification

- `kernel/src/syscall/signalfd.rs` line 221: Retracted the copy-failure/dequeued-signal finding because Linux consumes the pending signal after an `EFAULT` from `read(signalfd, bad_ptr, 128)`, so this behavior matches the compatibility target.
