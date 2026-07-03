---
date: 2026-07-03
mode: files
files: kernel/src/net/socket/unix/stream/connected.rs
head: cf90a0dc5-dirty
branch: HEAD
title: "Review kernel/src/net/socket/unix/stream/connected.rs"
---

# Summary

The most serious issues are in the stream/seqpacket auxiliary-data state machine. The `has_aux` fast-path flag can be observed outside the `all_aux` critical section, so a failed ancillary `sendmsg()` can make a concurrent receive return `Ok(0)` or trip the stream debug assertion instead of reporting no data. Separately, zero-length `SOCK_SEQPACKET` sends enqueue metadata without consuming ring-buffer capacity, giving an untrusted sender an unbounded kernel-memory/resource retention path, especially with `SCM_RIGHTS`.

The implementation does preserve several important invariants explicitly: the file documents the intended lock order, keeps unsafe code out of the kernel side, and uses `Endpoint` helpers for shutdown and readiness notification. The remaining maintainability comments are mostly about making the stream-versus-seqpacket split easier to audit and keeping comments accurate around `MSG_PEEK`.

The Linux compatibility docs also appear stale for this user-visible socket behavior: Unix stream receives now honor `MSG_PEEK`, and Unix stream sends accept ancillary data, while the SCML still documents narrower coverage.

## Maintainability

### `kernel/src/net/socket/unix/stream/connected.rs` line 119

> ```diff
> pub(super) fn try_read(
>     &self,
>     writer: &mut dyn MultiWrite,
>     is_seqpacket: bool,
>     flags: SendRecvFlags,
> ) -> Result<(usize, Vec<ControlMessage>)> {
> ```

`no-bool-args` (minor): `try_read()` takes `is_seqpacket` as a flag argument, and that flag selects different receive behavior for `SOCK_STREAM` versus `SOCK_SEQPACKET` in multiple branches. A call site passing `true` or `false` has to know this file's protocol-specific convention to understand which behavior is requested.

**Fix.** Shared with the other `is_seqpacket` boolean-argument comment: replace `is_seqpacket: bool` with a typed mode such as `UnixSocketKind::{Stream, SeqPacket}`, or split the public entry points into stream and seqpacket variants while sharing lower-level helpers for common buffer handling.

### `kernel/src/net/socket/unix/stream/connected.rs` line 171

> ```diff
> loop {
>     let read_start = read_base + Wrapping(read_tot_len);
> 
>     let (aux_len, aux_front) = if let Some(front) = all_aux.get(aux_pos) {
>         if front.start == read_start {
>             ((front.end - read_start).0, Some(front))
>         } else {
>             ((front.start - read_start).0, None)
>         }
>     } else {
>         ((reader.tail() - read_start).0, None)
>     };
> ```

`minimize-nesting` (major): The auxiliary-data slow path in `try_read()` nests the `loop`, `if let`, `if`, `match`, and later consume/truncate decisions in one block. This mixes range discovery, control-message compatibility, payload copying, `SOCK_SEQPACKET` truncation, and queue mutation, so the invariants for `aux_pos`, `read_tot_len`, and `trunc_len` have to be reconstructed inline.

**Fix.** Extract the slow path into focused helpers, for example `next_aux_segment()`, `can_extend_control_messages()`, `read_aux_segment()`, and `consume_aux_data()`, so `try_read()` shows the high-level receive flow and each helper owns one invariant.

### `kernel/src/net/socket/unix/stream/connected.rs` line 232

> ```diff
> // Record the current auxiliary data. Break if the read is incomplete or this is a
> // `SOCK_SEQPACKET` socket.
> if is_seqpacket {
> ```

`explain-why` (nit): The comment says this block records auxiliary data, but the code only handles incomplete reads and the `SOCK_SEQPACKET` boundary. That stale wording sends the reader looking for a state update that is not here.

**Fix.** Rewrite the comment to describe the decision being made, for example: `Stop after one packet, or after a partial stream read.`

### `kernel/src/net/socket/unix/stream/connected.rs` line 255

> ```diff
> // Consume the auxiliary data that we've read.
> let ctrl_msgs = if aux_pos >= 1
>     && let Some(aux_data) = all_aux.get_mut(aux_pos - 1)
> ```

`explain-why` (minor): The comment says this block consumes auxiliary data, but the block always builds `ctrl_msgs` and only mutates `all_aux` when `behavior.will_consume_data()` is true. For `MSG_PEEK`, the comment describes the opposite of what the code must preserve.

**Fix.** Make the comment match both modes, for example: `Build control messages, and advance the auxiliary-data queue only for consuming receives.`

### `kernel/src/net/socket/unix/stream/connected.rs` line 293

> ```diff
> pub(super) fn try_write(
>     &self,
>     reader: &mut dyn MultiRead,
>     aux_data: &mut AuxiliaryData,
>     is_seqpacket: bool,
> ) -> Result<usize> {
> ```

`no-bool-args` (minor): `try_write()` also uses `is_seqpacket` as a behavior-selecting flag, controlling empty writes, `EMSGSIZE`, the fast path, and whole-packet capacity checks. This keeps two socket write semantics inside one boolean-driven function.

**Fix.** Shared with the other `is_seqpacket` boolean-argument comment: use the same typed socket mode as `try_read()`, or split this into `try_write_stream()` and `try_write_seqpacket()` with shared helpers for the ring-buffer and auxiliary-data queue operations.

## Correctness

### `kernel/src/net/socket/unix/stream/connected.rs` line 142

> ```diff
> 141         // Fast path: There are no auxiliary data to receive.
> 142         if !peer_end.has_aux.load(Ordering::Relaxed) {
> ...
> 163         let mut all_aux = peer_end.all_aux.lock();
> ...
> 323         let mut all_aux = this_end.all_aux.lock();
> 324 
> 325         // No matter we succeed later or not, set the flag first to ensure that the auxiliary
> 326         // data are always visible to `try_recv`.
> 327         this_end.has_aux.store(true, Ordering::Relaxed);
> ...
> 343         let Ok(write_len) = write_res else {
> 344             this_end
> 345                 .has_aux
> 346                 .store(!all_aux.is_empty(), Ordering::Relaxed);
> 347             return write_res;
> ```

`atomic-critical-sections` (major): `try_read()` decides to use the auxiliary-data path from `peer_end.has_aux` before it holds `peer_end.all_aux`. But `try_write()` sets `has_aux` to `true` while `all_aux` is still empty, then can fail in `writer.write_fallible(reader)` and restore the flag without queuing any `RangedAuxiliaryData`. A concurrent reader that already observed `true` will block on `all_aux`, then see an empty queue and no payload, and return `Ok((0, ...))` on a non-shutdown socket (or hit the debug assertion for stream sockets) instead of `EAGAIN`.

**Fix.** Revalidate the condition after locking `all_aux`, or make `has_aux` publish only a state that is already represented in `all_aux`. For example, after acquiring `all_aux`, handle the `all_aux.is_empty()` case by falling back to the no-aux read path or returning `EAGAIN`/EOF through `Endpoint::read_with()` when there is no readable payload. Add a regression test where `sendmsg()` with ancillary data fails from a bad data iovec while a nonblocking peer `recvmsg()` runs concurrently.

## Security

### `kernel/src/net/socket/unix/stream/connected.rs` line 360

> ```diff
>         let aux_range = RangedAuxiliaryData {
>             data: core::mem::take(aux_data),
>             start: write_start,
>             end: write_start + Wrapping(write_len),
>         };
>         all_aux.push_back(aux_range);
> ```

`validate-at-boundaries` (critical): `try_write()` enqueues an `RangedAuxiliaryData` even when `write_len == 0`. On a `SOCK_SEQPACKET` UNIX socket, an untrusted caller can repeatedly call `sendmsg()` with `msg_iovlen == 0` or all zero-length iovecs; the ring buffer never consumes capacity, but `all_aux` grows without backpressure and can also keep `SCM_RIGHTS` file references alive, leading to unbounded kernel memory/resource exhaustion.

**Fix.** Do not enqueue metadata for a zero-length write unless zero-length packets are deliberately supported with bounded accounting. A minimal fix is to return before `all_aux.push_back()` when `write_len == 0`; if zero-length `SOCK_SEQPACKET` messages must be observable, charge each queued metadata record against a bounded receive-buffer/message limit and make `check_io_events()`/write readiness honor that limit.

## Documentation

### `kernel/src/net/socket/unix/stream/connected.rs` line 246

> ```diff
>    138	        let is_pass_cred = this_end.is_pass_cred.load(Ordering::Relaxed);
>    139	        let behavior = flags.receive_behavior();
> ...
>    245	        // Consume the payload bytes that we've read.
>    246	        if behavior.will_consume_data() {
>    247	            let consume_tot_len = read_tot_len + trunc_len;
>    248	            reader.commit_read(consume_tot_len);
> ```

`linux-compat-docs` (major): `try_read()` honors `SendRecvFlags::MSG_PEEK` through `flags.receive_behavior()` and skips `reader.commit_read()` when the behavior is peek, so Unix stream `recvfrom()`/`recvmsg()` now expose `MSG_PEEK`. The Linux compatibility docs still list `recvfrom()`/`recvmsg()` as `flags = 0` and say `MSG_PEEK` is only supported for netlink sockets, so the documented syscall flag coverage is stale.

**Fix.** Update `book/src/kernel/linux-compatibility/syscall-flag-coverage/networking-and-sockets/recvfrom_and_recvmsg.scml` and the adjacent `README.md` note to include `MSG_PEEK` for Unix stream sockets, or explicitly describe the socket-family limitation if support is narrower.

### `kernel/src/net/socket/unix/stream/connected.rs` line 355

> ```diff
>    289	    pub(super) fn try_write(
>    290	        &self,
>    291	        reader: &mut dyn MultiRead,
>    292	        aux_data: &mut AuxiliaryData,
> ...
>    354	        // Store the auxiliary data.
>    355	        let aux_range = RangedAuxiliaryData {
>    356	            data: core::mem::take(aux_data),
>    357	            start: write_start,
>    358	            end: write_start + Wrapping(write_len),
>    359	        };
>    360	        all_aux.push_back(aux_range);
> ```

`linux-compat-docs` (major): `try_write()` queues `AuxiliaryData` for Unix stream sockets, which makes `sendmsg()` with Unix ancillary data such as `SCM_RIGHTS` and `SCM_CREDENTIALS` part of the user-visible syscall surface. The networking SCML still defines `sendmsg()` with `msg_control = NULL`, so the Linux compatibility docs under-report supported ancillary-data behavior.

**Fix.** Update `book/src/kernel/linux-compatibility/syscall-flag-coverage/networking-and-sockets/sendto_and_sendmsg.scml` to document the supported Unix control messages, or add a partial-support note in the matching `README.md` if only some ancillary-data cases are supported.
