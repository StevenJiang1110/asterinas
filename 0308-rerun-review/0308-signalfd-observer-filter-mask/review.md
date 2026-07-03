---
date: 2026-07-03
mode: files
files: kernel/src/syscall/signalfd.rs,kernel/src/process/signal/events.rs,kernel/src/process/signal/sig_queues.rs
head: 20c2e967f
branch: HEAD
---

# Summary

The reviewed signal code has a clear split between queue mechanics and the new `signalfd` file interface, and the main flow is easy to follow. The serious issues are in the boundary between those pieces: observer filtering currently rejects the signals it should admit, `signalfd` observes one thread queue while reads use the current thread queue, and real-time pending checks ignore the caller's blocked mask. Those defects can produce hangs, lost wakeups, spurious signal interruptions, and incorrect user-visible sender metadata.

Top issues:

- Major: `SigEventsFilter` uses the inverted predicate, so matching signals do not wake `signalfd` waiters.
- Major: real-time pending checks ignore `blocked`, so blocked real-time signals can look deliverable.
- Major: `SignalFile` notification ownership does not match the queue used by `read()` and `poll()`.
- Major: `SignalfdSiginfo` drops sender `pid`/`uid` metadata.

## Correctness

### `kernel/src/process/signal/events.rs` line 35

> ```diff
> impl EventsFilter<SigEvents> for SigEventsFilter {
>     fn filter(&self, event: &SigEvents) -> bool {
>         !self.0.contains(event.0)
>     }
> }
> ```

Wrong predicate (major): `SigEventsFilter::filter()` returns `false` for signals that are in the registered `SigMask`, but `Subject::notify_observers()` only calls `on_events()` when `filter.filter(events)` is `true`. A `signalfd` registered for `SIGUSR1` therefore receives no notification when `SIGUSR1` is enqueued, so a blocking `read()` or `poll()` can sleep forever if the signal arrives after the waiter is registered.

**Fix.** Return `true` for events contained in the registered mask:

```rust
fn filter(&self, event: &SigEvents) -> bool {
    self.0.contains(event.0)
}
```

### `kernel/src/process/signal/sig_queues.rs` line 207

> ```diff
> fn has_pending(&self, blocked: SigMask) -> bool {
>     self.std_queues.iter().any(|signal| {
>         signal
>             .as_ref()
>             .is_some_and(|signal| !blocked.contains(signal.num()))
>     }) || self.rt_queues.iter().any(|rt_queue| !rt_queue.is_empty())
> }
> ```

Missing blocked check (major): `Queues::has_pending()` ignores `blocked` for real-time queues. With only a blocked real-time signal pending, this returns `true` even though `dequeue()` will skip that signal. `PosixThread::has_pending()` is used to interrupt waits and user-mode execution, so a blocked `SIGRTMIN` can cause spurious `EINTR` returns or repeated signal-handling exits with no deliverable signal.

**Fix.** Apply the same `blocked` predicate to real-time queues, deriving each queue's signal number from its index before treating a non-empty queue as pending.

### `kernel/src/syscall/signalfd.rs` line 124

> ```diff
> fn register_observer(ctx: &Context, signal_file: &Arc<SignalFile>, mask: SigMask) -> Result<()> {
>     let filter = SigEventsFilter::new(mask);
> 
>     ctx.posix_thread
>         .register_sigqueue_observer(signal_file.observer_ref(), filter);
> 
>     Ok(())
> }
> ```

Lost wakeup (major): `SignalFile` is registered only with `ctx.posix_thread`, but `check_io_events()` and `try_read()` read from `current_thread!()`. If a `signalfd` is inherited by a child or used by another thread, that thread can block in `poll()` after `check_io_events()` returns empty, then receive a matching signal in its own `SigQueues`; no observer is registered on that queue, so `SignalFile::on_events()` is never called and the waiter is not woken.

**Fix.** Make the observed signal queue match the queue used for reads. Either bind `SignalFile` to one owning `PosixThread` and always read that queue, or register/unregister the observer for each `PosixThread` that can block on this `SignalFile`, including cleanup for all registered queues.

### `kernel/src/syscall/signalfd.rs` line 353

> ```diff
> SignalfdSiginfo {
>     ssi_signo: siginfo.si_signo as _,
>     ssi_errno: siginfo.si_errno,
>     ssi_code: siginfo.si_code,
>     ssi_pid: 0,
>     ssi_uid: 0,
>     ssi_fd: 0,
>     ...
> }
> ```

Incorrect signal metadata (major): `to_signalfd_siginfo()` always reports `ssi_pid` and `ssi_uid` as `0`. For user-generated signals, `UserSignal` carries the sender `pid` and `uid`, and applications using `signalfd` rely on those fields to identify the sender; a `kill()` or `tgkill()` delivered through this path will return incorrect metadata.

**Fix.** Preserve sender metadata in `siginfo_t` or expose it through the `Signal` trait, then populate `SignalfdSiginfo` from that data instead of hard-coding `0`.

## Retracted by verification

- `kernel/src/syscall/signalfd.rs` line 46, `linux-compat-docs`: retracted because this is a files-mode review of the current checkout, and `git diff` shows no change to the reviewed syscall files. The documentation rule applies when a change under `kernel/` alters a user-visible API.
