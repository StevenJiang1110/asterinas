---
date: 2026-07-03
mode: files
files: kernel/src/net/socket/unix/stream/connected.rs
head: cf90a0dc5-dirty
branch: HEAD
title: "Review of kernel/src/net/socket/unix/stream/connected.rs"
---

# Summary

The connected UNIX stream code cleanly separates payload bytes from ancillary records, but two read-availability paths still treat the byte ring as the sole source of truth. The major risk is zero-length/control-only `SOCK_SEQPACKET` messages: they can be queued only in `all_aux`, while `try_read()` can take the no-aux fast path and `poll()`/`epoll()` can report no `IoEvents::IN`.

Fix both issues by making `all_aux` part of the readable-state contract. Either remove the relaxed `has_aux` shortcut, or make the empty-byte path verify the auxiliary queue under its mutex; then make readiness report `IoEvents::IN` for queued zero-length packet records.

## Correctness

### `kernel/src/net/socket/unix/stream/connected.rs` line 142

> ```diff
> 134        // `reader.len()` is an `Acquire` operation. So it can guarantee that the `has_aux`
> 135        // check below sees the up-to-date value.
> 136        let no_aux_len = reader.len();
> ...
> 141        // Fast path: There are no auxiliary data to receive.
> 142        if !peer_end.has_aux.load(Ordering::Relaxed) {
> ...
> 325        // No matter we succeed later or not, set the flag first to ensure that the auxiliary
> 326        // data are always visible to `try_recv`.
> 327        this_end.has_aux.store(true, Ordering::Relaxed);
> ```

`careful-atomics` (major): `has_aux` is a relaxed flag for state that actually lives in `all_aux`. For a zero-length `SOCK_SEQPACKET` or control-only message, `try_write()` enqueues an `all_aux` record without advancing the byte ring, so the `reader.len()` acquire load does not synchronize with `has_aux.store(true, Ordering::Relaxed)`. A receiver can observe stale `false`, take the fast path, and return `EAGAIN` even though a queued message is readable.

**Fix.** Shared with the other `all_aux` readability comment: make the auxiliary queue part of the read-availability protocol. The robust fix is to remove the relaxed fast-path flag, or at least fall back to checking `peer_end.all_aux` under its mutex whenever no payload bytes are visible, so zero-byte/control-only packets cannot be hidden by a stale `has_aux` load.

### `kernel/src/net/socket/unix/stream/connected.rs` line 400

> ```diff
> 396    pub(super) fn check_io_events(&self) -> IoEvents {
> 397        let this_end = self.inner.this_end();
> 398        let mut events = IoEvents::empty();
> 399
> 400        if !this_end.reader.lock().is_empty() {
> 401            events |= IoEvents::IN;
> 402        }
> ```

Missed readiness (major): `check_io_events()` reports `IoEvents::IN` only when the byte ring is non-empty. Zero-length `SOCK_SEQPACKET` messages, including `sendmsg()` with only `SCM_RIGHTS`, are represented by an `all_aux` record with `start == end` and no bytes in `reader`, so `poll()`/`epoll()` can report no readable event while `recvmsg()` would consume a queued packet.

**Fix.** Shared with the other `all_aux` readability comment: make readability include queued zero-length packet records, e.g. pass `is_seqpacket` into `Connected::check_io_events()` and set `IoEvents::IN` when `self.inner.peer_end().all_aux` contains a record at the current `reader.head()` even if `reader.is_empty()`.
