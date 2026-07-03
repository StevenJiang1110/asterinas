---
date: 2026-07-02
mode: files
files: kernel/src/syscall/signalfd.rs,kernel/src/process/signal/events.rs,kernel/src/process/signal/sig_queues.rs
head: 20c2e967f
branch: HEAD
title: "Review of signalfd signal queue files"
---

# Summary

The reviewed signal/signalfd code is close to the Linux API shape, but the main defects are in readiness and pending-signal semantics. The highest-priority issues are the inverted `SigEventsFilter`, which can leave pollers asleep after a watched signal arrives, and the real-time pending check that ignores blocked masks and can spuriously interrupt waits. The signalfd update path also changes `O_NONBLOCK` while replacing only the signal mask should be enough, and the exported `signalfd_siginfo` currently loses sender identity fields that user space may rely on.

Structurally, the code would benefit from separating "signals accepted by this signalfd" naming from the existing `SigMask` convention of "blocked signals"; that ambiguity appears directly related to the predicate bugs below.

## Maintainability

### `kernel/src/process/signal/events.rs` line 35

> ```diff
> 33 impl EventsFilter<SigEvents> for SigEventsFilter {
> 34     fn filter(&self, event: &SigEvents) -> bool {
> 35         !self.0.contains(event.0)
> 36     }
> 37 }
> ```

`bug` (major): `Subject::notify_observers` calls observers only when `filter.filter(events)` returns true, but `SigEventsFilter` returns false for signals contained in the registered mask. A signalfd registered for SIGUSR1 will therefore not be notified when SIGUSR1 is enqueued after `poll` has gone to sleep, so blocking `poll`/`read` can hang even though a watched signal is pending.

**Fix.** Shared with the Correctness comment at `kernel/src/process/signal/events.rs` line 35: make `SigEventsFilter` admit events whose signal is in the registered mask, i.e. `self.0.contains(event.0)`.

### `kernel/src/process/signal/sig_queues.rs` line 207

> ```diff
> 201     /// Returns whether the `SigQueues` has some pending signals which are not blocked
> 202     fn has_pending(&self, blocked: SigMask) -> bool {
> 203         self.std_queues.iter().any(|signal| {
> 204             signal
> 205                 .as_ref()
> 206                 .is_some_and(|signal| !blocked.contains(signal.num()))
> 207         }) || self.rt_queues.iter().any(|rt_queue| !rt_queue.is_empty())
> 208     }
> ```

`bug` (major): The real-time signal branch ignores the `blocked` mask. If SIGRTMIN is pending and blocked, `has_pending` still returns true even though `dequeue` will refuse to deliver it; callers such as `Pause` can then spuriously return EINTR, and the user-mode event loop can repeatedly wake for an undeliverable signal.

**Fix.** Shared with the Correctness comment at `kernel/src/process/signal/sig_queues.rs` line 207: apply the same blocked-signal test to real-time queues, deriving the queue's signal number from the index before returning true.

### `kernel/src/syscall/signalfd.rs` line 117

> ```diff
> 75     let non_blocking = flags.contains(SignalFileFlags::O_NONBLOCK);
> ...
> 80         update_existing_signalfd(ctx, fd, mask, non_blocking)?
> ...
> 114     if signal_file.mask().load(Ordering::Relaxed) != new_mask {
> 115         signal_file.update_signal_mask(new_mask)?;
> 116     }
> 117     signal_file.set_non_blocking(non_blocking);
> ```

`bug` (major): Updating an existing signalfd rewrites its nonblocking status from the current syscall's `flags`. A concrete failure is `signalfd(-1, mask, SFD_NONBLOCK)` followed by `signalfd(fd, new_mask, size)`: the second call passes flags 0 through `sys_signalfd`, this line clears `O_NONBLOCK`, and a later read can block unexpectedly. The update path should replace only the signal mask.

**Fix.** Do not pass `non_blocking` to `update_existing_signalfd`, and remove `signal_file.set_non_blocking(non_blocking)` from the existing-fd path.

### `kernel/src/syscall/signalfd.rs` line 144

> ```diff
> 142 struct SignalFile {
> 143     /// Atomic signal mask for filtering signals
> 144     signals_mask: AtomicSigMask,
> 145     /// I/O event notifier
> 146     pollee: Pollee,
> ```

`accurate-names` (minor): `signals_mask` is stored as `AtomicSigMask`, whose local documentation means a thread's blocked-signal mask, but this field actually stores the signals accepted by the signalfd. The misleading name/type pairing obscures why `try_read` must pass the complement to `dequeue_signal` and makes inverted predicates like the `SigEventsFilter` bug easy to write.

**Fix.** Rename the field and accessors to reflect signalfd semantics, for example `accepted_signals` or `signalfd_mask`, and update the comment to say it is the set delivered through this file descriptor, not the thread blocked mask.

## Correctness

### `kernel/src/process/signal/events.rs` line 35

> ```diff
> impl EventsFilter<SigEvents> for SigEventsFilter {
>     fn filter(&self, event: &SigEvents) -> bool {
>         !self.0.contains(event.0)
>     }
> }
> ```

`bug` (major): The signal-event filter is inverted. `Subject::notify_observers` calls `on_events` only when `filter.filter(events)` returns true, but this implementation returns true for signals outside the registered signalfd mask. For a concrete failure, create a signalfd for SIGUSR1, poll it while no SIGUSR1 is pending, then deliver SIGUSR1: the observer is filtered out before `SignalFile::on_events`, so the poller is never notified and the blocking poll can sleep indefinitely.

**Fix.** Shared with the Maintainability comment at `kernel/src/process/signal/events.rs` line 35: make the filter accept signals contained in the registered mask, i.e. `self.0.contains(event.0)`.

### `kernel/src/process/signal/sig_queues.rs` line 207

> ```diff
> }) || self.rt_queues.iter().any(|rt_queue| !rt_queue.is_empty())
> ```

`bug` (major): `Queues::has_pending` ignores the blocked mask for real-time signals. If SIGRTMIN is blocked and one SIGRTMIN is pending, the standard-signal half returns false but this real-time half returns true just because the queue is nonempty; callers such as `Pause::pause_until_or_timeout_impl` then report EINTR even though there is no unblocked pending signal.

**Fix.** Shared with the Maintainability comment at `kernel/src/process/signal/sig_queues.rs` line 207: apply the same blocked-mask predicate to real-time queues by deriving each queue's signal number from its index before treating it as pending.

## Security

### `kernel/src/syscall/signalfd.rs` line 353

> ```diff
>             ssi_code: siginfo.si_code,
>             ssi_pid: 0,
>             ssi_uid: 0,
>             ssi_fd: 0,
> ```

`bug` (major): The signalfd ABI hard-codes all sender identity fields to zero. A user signal sent by an untrusted process will be reported to readers as `ssi_pid = 0` and `ssi_uid = 0`, so a privileged program that relies on signalfd's documented sender credentials can mistake the sender for root or for an absent PID.

**Fix.** Preserve the metadata from the underlying signal instead of zeroing it. Add safe accessors on `siginfo_t` for the pid/uid/value/address fields and make `UserSignal::to_info` populate them, then copy those fields here into `SignalfdSiginfo` rather than using literal zeroes.

## Documentation

No findings.

## Retracted by verification

- `kernel/src/syscall/signalfd.rs` line 46, documentation/linux-compat-docs: refuted because `docs/src/kernel/linux-compatibility.md` already lists both `signalfd` and `signalfd4` as supported.
