// SPDX-License-Identifier: MPL-2.0

#![no_std]

extern crate alloc;

use alloc::sync::Arc;
use core::{
    marker::PhantomData,
    num::Wrapping,
    ops::Deref,
    sync::atomic::{AtomicUsize, Ordering},
};

use aster_util::{MultiRead, MultiWrite};
use inherit_methods_macro::inherit_methods;
use ostd::mm::{FrameAllocOptions, PAGE_SIZE, Segment, VmIo, io::util::HasVmReaderWriter};
use ostd_pod::Pod;

/// A lock-free single-producer single-consumer (SPSC) FIFO ring buffer.
///
/// This ring buffer is backed by a [`Segment<()>`] and provides non-blocking
/// `push`/`pop` and `push_slice`/`pop_slice` operations for `T: Pod` items.
/// It is designed for concurrent use where one thread produces items and
/// another consumes them without requiring locks.
///
/// # Constraints
///
/// - The capacity must be a power of two.
/// - Items must implement the [`Pod`] trait for safe memory operations.
///
/// # Usage Patterns
///
/// For concurrent SPSC usage, call [`split`](Self::split) to obtain a
/// [`Producer`] and [`Consumer`] pair that can be safely used from
/// different threads. For single-threaded usage, the `push`/`pop` methods
/// can be called directly on a mutable reference.
///
/// # Example
///
/// ```ignore
/// use ring_buffer::RingBuffer;
///
/// let rb = RingBuffer::<u8>::new(16);
/// let (mut producer, mut consumer) = rb.split();
///
/// producer.push(42).unwrap();
/// assert_eq!(consumer.pop(), Some(42));
/// ```
pub struct RingBuffer<T> {
    segment: Segment<()>,
    capacity: usize,
    tail: AtomicUsize,
    head: AtomicUsize,
    phantom: PhantomData<T>,
}

/// The producer half of a [`RingBuffer`].
///
/// A `Producer` has exclusive rights to push items into the ring buffer.
/// It can be safely used from one thread while a [`Consumer`] operates
/// on the same ring buffer from another thread.
pub struct Producer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}

/// The consumer half of a [`RingBuffer`].
///
/// A `Consumer` has exclusive rights to pop items from the ring buffer.
/// It can be safely used from one thread while a [`Producer`] operates
/// on the same ring buffer from another thread.
pub struct Consumer<T, R: Deref<Target = RingBuffer<T>>> {
    rb: R,
    phantom: PhantomData<T>,
}

/// A producer backed by an `Arc<RingBuffer<T>>`.
pub type RbProducer<T> = Producer<T, Arc<RingBuffer<T>>>;

/// A consumer backed by an `Arc<RingBuffer<T>>`.
pub type RbConsumer<T> = Consumer<T, Arc<RingBuffer<T>>>;

impl<T> RingBuffer<T> {
    const T_SIZE: usize = size_of::<T>();

    /// Creates a new ring buffer with the specified capacity.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is not a power of two.
    pub fn new(capacity: usize) -> Self {
        assert!(
            capacity.is_power_of_two(),
            "capacity must be a power of two"
        );

        let nframes = capacity
            .checked_mul(Self::T_SIZE)
            .unwrap()
            .div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment(nframes)
            .unwrap();

        Self {
            segment,
            capacity,
            tail: AtomicUsize::new(0),
            head: AtomicUsize::new(0),
            phantom: PhantomData,
        }
    }

    /// Splits the ring buffer into a producer and consumer pair.
    ///
    /// The returned [`RbProducer`] and [`RbConsumer`] share ownership of the
    /// underlying buffer via `Arc` and can be used concurrently from different threads.
    pub fn split(self) -> (RbProducer<T>, RbConsumer<T>) {
        let producer = Producer {
            rb: Arc::new(self),
            phantom: PhantomData,
        };
        let consumer = Consumer {
            rb: Arc::clone(&producer.rb),
            phantom: PhantomData,
        };
        (producer, consumer)
    }

    /// Returns the capacity of the ring buffer in number of items.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Returns a reference to the underlying memory segment.
    ///
    /// This is intended for advanced use cases that require direct memory access.
    pub fn segment(&self) -> &Segment<()> {
        &self.segment
    }

    /// Returns `true` if the ring buffer contains no items.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Returns `true` if the ring buffer is at full capacity.
    pub fn is_full(&self) -> bool {
        self.free_len() == 0
    }

    /// Returns the number of items currently in the ring buffer.
    pub fn len(&self) -> usize {
        // Implementation notes: This subtraction only makes sense if either the head or the tail
        // is considered frozen; if both are volatile, the number of the items may become negative
        // due to race conditions. This is always true with a `RingBuffer` or a pair of
        // `RbProducer` and `RbConsumer`.
        (self.tail() - self.head()).0
    }

    /// Returns the number of items that can be pushed before the buffer is full.
    pub fn free_len(&self) -> usize {
        self.capacity - self.len()
    }

    /// Returns the head counter value.
    ///
    /// This represents the cumulative number of items that have been read from
    /// the ring buffer since creation. The value wraps on overflow.
    pub fn head(&self) -> Wrapping<usize> {
        Wrapping(self.head.load(Ordering::Acquire))
    }

    /// Returns the tail counter value.
    ///
    /// This represents the cumulative number of items that have been written to
    /// the ring buffer since creation. The value wraps on overflow.
    pub fn tail(&self) -> Wrapping<usize> {
        Wrapping(self.tail.load(Ordering::Acquire))
    }

    /// Advances the tail by `len` items starting from `tail`.
    ///
    /// This is an internal method. External users should use the safe
    /// `commit_write` method on `Producer` instead.
    pub(crate) fn advance_tail(&self, mut tail: Wrapping<usize>, len: usize) {
        tail += len;
        self.tail.store(tail.0, Ordering::Release);
    }

    /// Advances the head by `len` items starting from `head`.
    ///
    /// This is an internal method. External users should use the safe
    /// `commit_read` method on `Consumer` instead.
    pub(crate) fn advance_head(&self, mut head: Wrapping<usize>, len: usize) {
        head += len;
        self.head.store(head.0, Ordering::Release);
    }

    /// Resets the head to the current tail, effectively draining the buffer.
    ///
    /// This is an internal method. External users should use `Consumer::clear` instead.
    pub(crate) fn reset_head(&self) {
        let new_head = self.tail();
        self.head.store(new_head.0, Ordering::Release);
    }

    /// Resets the ring buffer to an empty state.
    ///
    /// This method requires exclusive access (`&mut self`) and should only be
    /// called when no concurrent producers or consumers are operating on the buffer.
    pub fn clear(&mut self) {
        self.tail.store(0, Ordering::Release);
        self.head.store(0, Ordering::Release);
    }
}

impl RingBuffer<u8> {
    /// Commits a read operation by advancing the head pointer.
    ///
    /// This method is intended for advanced use cases where the caller reads
    /// data directly from the backing segment and needs to update the head.
    /// For normal use, prefer `Consumer::pop` or `Consumer::pop_slice`.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the number of available items in the buffer.
    pub fn commit_read(&mut self, len: usize) {
        assert!(
            len <= self.len(),
            "commit_read: len exceeds available items"
        );
        let head = self.head();
        self.advance_head(head, len);
    }
}

impl<T: Pod> RingBuffer<T> {
    /// Pushes a single item into the ring buffer.
    ///
    /// Returns `Some(())` on success, or `None` if the buffer is full.
    pub fn push(&mut self, item: T) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push(item)
    }

    /// Pushes all items from the slice into the ring buffer.
    ///
    /// Returns `Some(())` if all items were successfully pushed, or `None` if
    /// there is not enough free space to fit all items. This is an all-or-nothing
    /// operation; no items are pushed if the slice cannot fit entirely.
    pub fn push_slice(&mut self, items: &[T]) -> Option<()> {
        let mut producer = Producer {
            rb: self,
            phantom: PhantomData,
        };
        producer.push_slice(items)
    }

    /// Pops a single item from the ring buffer.
    ///
    /// Returns `Some(item)` on success, or `None` if the buffer is empty.
    pub fn pop(&mut self) -> Option<T> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.pop()
    }

    /// Pops items from the ring buffer into the provided slice.
    ///
    /// Returns `Some(())` if all slots in the slice were filled, or `None` if
    /// there are not enough items available. This is an all-or-nothing operation;
    /// no items are popped if the slice cannot be filled entirely.
    pub fn pop_slice(&mut self, items: &mut [T]) -> Option<()> {
        let mut consumer = Consumer {
            rb: self,
            phantom: PhantomData,
        };
        consumer.pop_slice(items)
    }
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    const T_SIZE: usize = size_of::<T>();

    /// Pushes a single item into the ring buffer.
    ///
    /// Returns `Some(())` on success, or `None` if the buffer is full.
    pub fn push(&mut self, item: T) -> Option<()> {
        let rb = &self.rb;
        if rb.is_full() {
            return None;
        }

        let tail = rb.tail();
        let offset = tail.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        let mut writer = rb.segment.writer();
        writer.skip(byte_offset);
        writer.write_val(&item).unwrap();

        rb.advance_tail(tail, 1);
        Some(())
    }

    /// Pushes all items from the slice into the ring buffer.
    ///
    /// Returns `Some(())` if all items were successfully pushed, or `None` if
    /// there is not enough free space. This is an all-or-nothing operation;
    /// no items are pushed if the slice cannot fit entirely.
    pub fn push_slice(&mut self, items: &[T]) -> Option<()> {
        let rb = &self.rb;
        let nitems = items.len();
        if rb.free_len() < nitems {
            return None;
        }

        let tail = rb.tail();
        let offset = tail.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        if offset + nitems > rb.capacity {
            // Write into two separate parts due to wraparound.
            rb.segment
                .write_slice(byte_offset, &items[..rb.capacity - offset])
                .unwrap();
            rb.segment
                .write_slice(0, &items[rb.capacity - offset..])
                .unwrap();
        } else {
            rb.segment.write_slice(byte_offset, items).unwrap();
        }

        rb.advance_tail(tail, nitems);
        Some(())
    }
}

#[inherit_methods(from = "self.rb")]
impl<T, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    pub fn capacity(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn free_len(&self) -> usize;
    pub fn head(&self) -> Wrapping<usize>;
    pub fn tail(&self) -> Wrapping<usize>;
    pub fn segment(&self) -> &Segment<()>;
}

impl<T, R: Deref<Target = RingBuffer<T>>> Producer<T, R> {
    /// Commits a write operation by advancing the tail pointer.
    ///
    /// This method is intended for advanced use cases where the caller writes
    /// data directly to the backing segment and needs to update the tail.
    /// For normal use, prefer `Producer::push` or `Producer::push_slice`.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the available free space in the buffer.
    pub fn commit_write(&self, len: usize) {
        assert!(
            len <= self.free_len(),
            "commit_write: len exceeds free space"
        );
        let tail = self.tail();
        self.rb.advance_tail(tail, len);
    }
}

impl<R: Deref<Target = RingBuffer<u8>>> Producer<u8, R> {
    /// Writes data from `reader` to the ring buffer.
    ///
    /// Returns the number of bytes written.
    pub fn write_fallible(&mut self, reader: &mut dyn MultiRead) -> ostd::Result<usize> {
        self.write_fallible_with_max_len(reader, usize::MAX)
    }

    /// Writes up to `max_len` bytes from `reader` to the ring buffer.
    ///
    /// Returns the number of bytes written.
    pub fn write_fallible_with_max_len(
        &mut self,
        reader: &mut dyn MultiRead,
        max_len: usize,
    ) -> ostd::Result<usize> {
        let free_len = self.free_len().min(max_len);

        let tail = self.tail();
        let offset = tail.0 & (self.capacity() - 1);

        let write_result = if offset + free_len > self.capacity() {
            let first_len = self.capacity() - offset;

            let mut writer = self.segment().writer();
            writer.skip(offset).limit(first_len);
            let first_write_len = match reader.read(&mut writer) {
                Ok(write_len) => write_len,
                Err((err, write_len)) => {
                    self.commit_write(write_len);
                    return Err(err);
                }
            };

            let mut writer = self.segment().writer();
            writer.limit(free_len - first_len);
            reader
                .read(&mut writer)
                .map(|second_write_len| first_write_len + second_write_len)
                .map_err(|(err, second_write_len)| (err, first_write_len + second_write_len))
        } else {
            let mut writer = self.segment().writer();
            writer.skip(offset).limit(free_len);
            reader.read(&mut writer)
        };

        match write_result {
            Ok(write_len) => {
                self.commit_write(write_len);
                Ok(write_len)
            }
            Err((err, write_len)) => {
                self.commit_write(write_len);
                Err(err)
            }
        }
    }
}

impl<T: Pod, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    const T_SIZE: usize = size_of::<T>();

    /// Pops a single item from the ring buffer.
    ///
    /// Returns `Some(item)` on success, or `None` if the buffer is empty.
    pub fn pop(&mut self) -> Option<T> {
        let rb = &self.rb;
        if rb.is_empty() {
            return None;
        }

        let head = rb.head();
        let offset = head.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        let mut reader = rb.segment.reader();
        reader.skip(byte_offset);
        let item = reader.read_val::<T>().unwrap();

        rb.advance_head(head, 1);
        Some(item)
    }

    /// Pops items from the ring buffer into the provided slice.
    ///
    /// Returns `Some(())` if all slots in the slice were filled, or `None` if
    /// there are not enough items available. This is an all-or-nothing operation;
    /// no items are popped if the slice cannot be filled entirely.
    pub fn pop_slice(&mut self, items: &mut [T]) -> Option<()> {
        let rb = &self.rb;
        let nitems = items.len();
        if rb.len() < nitems {
            return None;
        }

        let head = rb.head();
        let offset = head.0 & (rb.capacity - 1);
        let byte_offset = offset * Self::T_SIZE;

        if offset + nitems > rb.capacity {
            // Read from two separate parts due to wraparound.
            rb.segment
                .read_slice(byte_offset, &mut items[..rb.capacity - offset])
                .unwrap();
            rb.segment
                .read_slice(0, &mut items[rb.capacity - offset..])
                .unwrap();
        } else {
            rb.segment.read_slice(byte_offset, items).unwrap();
        }

        rb.advance_head(head, nitems);
        Some(())
    }

    /// Discards `count` items from the ring buffer without reading them.
    ///
    /// # Panics
    ///
    /// Panics if `count` exceeds the number of available items in the buffer.
    pub fn skip(&mut self, count: usize) {
        let rb = &self.rb;
        let len = rb.len();
        assert!(len >= count, "skip: count exceeds available items");

        let head = rb.head();
        rb.advance_head(head, count);
    }

    /// Discards all items from the ring buffer.
    ///
    /// After this call, the buffer will be empty from the consumer's perspective.
    pub fn clear(&mut self) {
        self.rb.reset_head();
    }
}

#[inherit_methods(from = "self.rb")]
impl<T, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    pub fn capacity(&self) -> usize;
    pub fn is_empty(&self) -> bool;
    pub fn is_full(&self) -> bool;
    pub fn len(&self) -> usize;
    pub fn free_len(&self) -> usize;
    pub fn head(&self) -> Wrapping<usize>;
    pub fn tail(&self) -> Wrapping<usize>;
    pub fn segment(&self) -> &Segment<()>;
}

impl<T, R: Deref<Target = RingBuffer<T>>> Consumer<T, R> {
    /// Commits a read operation by advancing the head pointer.
    ///
    /// This method is intended for advanced use cases where the caller reads
    /// data directly from the backing segment and needs to update the head.
    /// For normal use, prefer `Consumer::pop` or `Consumer::pop_slice`.
    ///
    /// # Panics
    ///
    /// Panics if `len` exceeds the number of available items in the buffer.
    pub fn commit_read(&self, len: usize) {
        assert!(
            len <= self.len(),
            "commit_read: len exceeds available items"
        );
        let head = self.head();
        self.rb.advance_head(head, len);
    }
}

impl<R: Deref<Target = RingBuffer<u8>>> Consumer<u8, R> {
    /// Reads data from the ring buffer into `writer`.
    ///
    /// Returns the number of bytes read.
    pub fn read_fallible(&mut self, writer: &mut dyn MultiWrite) -> ostd::Result<usize> {
        self.read_fallible_with_max_len(writer, usize::MAX)
    }

    /// Reads up to `max_len` bytes from the ring buffer into `writer`.
    ///
    /// Returns the number of bytes read.
    pub fn read_fallible_with_max_len(
        &mut self,
        writer: &mut dyn MultiWrite,
        max_len: usize,
    ) -> ostd::Result<usize> {
        let len = self.len().min(max_len);

        let head = self.head();
        let offset = head.0 & (self.capacity() - 1);

        let read_result = if offset + len > self.capacity() {
            let first_len = self.capacity() - offset;

            let mut reader = self.segment().reader();
            reader.skip(offset).limit(first_len);
            let first_read_len = match writer.write(&mut reader) {
                Ok(read_len) => read_len,
                Err((err, read_len)) => {
                    self.commit_read(read_len);
                    return Err(err);
                }
            };

            let mut reader = self.segment().reader();
            reader.limit(len - first_len);
            writer
                .write(&mut reader)
                .map(|second_read_len| first_read_len + second_read_len)
                .map_err(|(err, second_read_len)| (err, first_read_len + second_read_len))
        } else {
            let mut reader = self.segment().reader();
            reader.skip(offset).limit(len);
            writer.write(&mut reader)
        };

        match read_result {
            Ok(read_len) => {
                self.commit_read(read_len);
                Ok(read_len)
            }
            Err((err, read_len)) => {
                self.commit_read(read_len);
                Err(err)
            }
        }
    }
}

#[cfg(ktest)]
mod test;
