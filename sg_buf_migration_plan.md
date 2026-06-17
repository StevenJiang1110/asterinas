# MultiRead/MultiWrite Migration Plan

## Goal

Move reusable scatter-gather buffer APIs out of `aster-kernel` and into
`aster-util`, so kernel components and utility crates can share byte-copying
interfaces without depending on `aster-kernel`.

The migration is split into self-contained commits. Each commit should compile
on its own.

## Commit 1: Move ReadCString API

Move `ReadCString` before moving `MultiRead`, so the existing
`MultiRead: ReadCString` relationship can be preserved after `MultiRead` moves
to `aster-util`.

1. Add `kernel/libs/aster-util/src/read_cstring.rs`.
2. Move the `ReadCString` trait and the `VmReader<'_, Fallible>`
   implementation to `aster-util`.
3. Move the shared NUL-byte scanning helper to `aster-util`.
4. Keep the `VmReaderArray<'_>` implementation in `aster-kernel`, because
   `VmReaderArray` is still a kernel-local type.
5. Re-export `ReadCString` from `kernel/src/util/mod.rs` to preserve existing
   kernel imports.

## Commit 2: Move Scatter-Gather Buffer Traits

Move `MultiRead` and `MultiWrite` from `kernel/src/util/iovec.rs` to
`aster-util`.

1. Add `kernel/libs/aster-util/src/sg_buf.rs`.
2. Move the `MultiRead` and `MultiWrite` trait definitions to `aster-util`.
3. Keep `MultiRead: ReadCString`.
4. Implement the moved traits in the crate allowed by Rust's orphan rules:
   - In `aster-util`, implement `MultiRead` for `VmReader<'_>`.
   - In `aster-util`, implement `MultiWrite` for `VmWriter<'_>`.
   - In `aster-kernel`, keep `MultiRead` for `VmReaderArray<'_>`.
   - In `aster-kernel`, keep `MultiWrite` for `VmWriterArray<'_>`.
5. Move the trait-object helper methods into `aster-util`:
   - `impl dyn MultiRead + '_ { read_val_opt(...) }`
   - `impl dyn MultiWrite + '_ { write_val_trunc(...) }`
6. Re-export `MultiRead` and `MultiWrite` from `kernel/src/util/mod.rs`.

## Commit 3: Move Ring Buffer Byte I/O API

Move the byte-oriented ring-buffer copy helpers from kernel-only extension
traits into the `ring-buffer` crate.

1. Add `aster-util.workspace = true` to `kernel/libs/ring-buffer/Cargo.toml`.
2. Move the byte-copy helpers into inherent methods on:
   - `Producer<u8, R>`
   - `Consumer<u8, R>`
3. Remove the kernel-only `ProducerU8Ext` and `ConsumerU8Ext` traits.
4. Update kernel call sites to use the new inherent methods.
5. Preserve existing copy semantics:
   - Copy at most the ring buffer free/available length and `max_len`.
   - Split copies when the ring buffer wraps.
   - Commit only bytes actually copied.
   - Commit partial-copy progress before returning a page-fault error.
6. Keep the new APIs returning `ostd::Result<usize>`.

## Validation

Each commit should pass:

```bash
make format
make kernel
make check
```
