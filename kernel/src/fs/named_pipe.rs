// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering};

use ostd::sync::WaitQueue;

use super::pipe::{self, PipeReader, PipeWriter};
use crate::{
    events::IoEvents,
    fs::{
        inode_handle::FileIo,
        utils::{AccessMode, StatusFlags},
    },
    prelude::*,
    process::{posix_thread::AsPosixThread, signal::{constants::SIGPIPE, signals::kernel::KernelSignal, PollHandle, Pollable}},
};

/// A handle representing the reader end of a named pipe.
struct ReadHandle {
    inner: Arc<(PipeReader, PipeWriter)>,
}

impl Pollable for ReadHandle {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.inner
            .0
            .state
            .poll_with(mask, poller, || self.inner.0.check_io_events())
    }
}

impl Drop for ReadHandle {
    fn drop(&mut self) {
        println!("drop read handle");
        self.inner.0.state.peer_shutdown();
    }
}

/// A handle representing the writer end of a named pipe.
struct WriteHandle {
    inner: Arc<(PipeReader, PipeWriter)>,
}

impl Pollable for WriteHandle {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.inner
            .1
            .state
            .poll_with(mask, poller, || self.inner.1.check_io_events())
    }
}

impl Drop for WriteHandle {
    fn drop(&mut self) {
        println!("drop write handle");
        self.inner.1.state.shutdown();
    }
}

enum HandleInner {
    Reader(Arc<ReadHandle>),
    Writer(Arc<WriteHandle>),
    ReaderWriter(Arc<ReadHandle>, Arc<WriteHandle>),
}

/// A handle for a named pipe that implements `FileIo`.
///
/// It contains either a reader, a writer, or both. Once a handle for a `NamedPipe` exists,
/// the corresponding pipe pair will not be dropped.
struct NamedPipeHandle {
    inner: HandleInner,
    // TODO: Currently this `status_flags` is duplicated with the one in `InodeHandle_`.
    // We need further refactoring to find an appropriate way to enable `FileIo` to utilize the
    // information in the `InodeHandle_`.
    status_flags: AtomicU32,
}

impl NamedPipeHandle {
    fn new(inner: HandleInner, status_flags: StatusFlags) -> Arc<Self> {
        Arc::new(Self {
            inner,
            status_flags: AtomicU32::new(status_flags.bits()),
        })
    }

    fn read_nonblocking(&self, writer: &mut VmWriter) -> Result<usize> {
        match &self.inner {
            HandleInner::Reader(handle) => handle.inner.0.try_read(writer),
            HandleInner::Writer(_) => {
                Err(Error::with_message(Errno::EBADF, "not opened for reading"))
            }
            HandleInner::ReaderWriter(reader, _) => reader.inner.0.try_read(writer),
        }
    }

    fn write_nonblocking(&self, reader: &mut VmReader) -> Result<usize> {
        match &self.inner {
            HandleInner::Reader(_) => {
                Err(Error::with_message(Errno::EBADF, "not opened for writing"))
            }
            HandleInner::Writer(handle) => {
                handle.inner.1.try_write(reader)
            }
            HandleInner::ReaderWriter(_, writer) => writer.inner.1.try_write(reader),
        }
    }

    fn status_flags(&self) -> StatusFlags {
        StatusFlags::from_bits_truncate(self.status_flags.load(Ordering::Relaxed))
    }
}

impl Pollable for NamedPipeHandle {
    fn poll(&self, mask: IoEvents, mut poller: Option<&mut PollHandle>) -> IoEvents {
        match &self.inner {
            HandleInner::Reader(handle) => handle.poll(mask, poller),
            HandleInner::Writer(handle) => handle.poll(mask, poller),
            HandleInner::ReaderWriter(reader, writer) => {
                reader.poll(mask, poller.as_deref_mut()) | writer.poll(mask, poller)
            }
        }
    }
}

impl FileIo for NamedPipeHandle {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        if self.status_flags().contains(StatusFlags::O_NONBLOCK) {
            self.read_nonblocking(writer)
        } else {
            self.wait_events(IoEvents::IN, None, || self.read_nonblocking(writer))
        }
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        if self.status_flags().contains(StatusFlags::O_NONBLOCK) {
            self.write_nonblocking(reader)
        } else {
            self.wait_events(IoEvents::OUT, None, || self.write_nonblocking(reader))
        }
    }

    fn set_status_flags(&self, status_flags: StatusFlags) {
        self.status_flags
            .store(status_flags.bits(), Ordering::Relaxed);
    }
}

/// A named pipe (FIFO) that provides inter-process communication.
///
/// Named pipes are special files that appear in the filesystem and provide
/// a communication channel between processes. It can be opened multiple times
/// for reading, writing, or both.
pub struct NamedPipe {
    pipe: Mutex<NamedPipeInner>,
    wait_queue: WaitQueue,
}

impl NamedPipe {
    pub fn new() -> Result<Self> {
        Ok(Self {
            pipe: Mutex::new(NamedPipeInner::default()),
            wait_queue: WaitQueue::new(),
        })
    }

    /// Opens the named pipe with the specified access mode and status flags.
    ///
    /// Returns a handle that implements `FileIo` for performing I/O operations.
    ///
    /// The open behavior follows POSIX semantics:
    /// - Opening for read-only blocks until a writer opens the pipe.
    /// - Opening for write-only blocks until a reader opens the pipe.
    /// - Opening for read-write never blocks.
    ///
    /// If no handle of this named pipe has existed, the method will create a new pipe pair.
    /// Otherwise, it will return a handle that works on the existing pipe pair.
    pub fn open(
        &self,
        access_mode: AccessMode,
        status_flag: StatusFlags,
    ) -> Result<Arc<dyn FileIo>> {
        let mut pipe = self.pipe.lock();
        let handle: Arc<dyn FileIo> = match access_mode {
            AccessMode::O_RDONLY => {
                let reader = pipe.get_or_create_reader();

                self.wait_queue.wake_all();

                if !status_flag.contains(StatusFlags::O_NONBLOCK) && !pipe.has_write_handle() {
                    let old_write_count = pipe.write_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_write_count != self.pipe.lock().write_count).then_some(())
                    })?;
                }

                NamedPipeHandle::new(HandleInner::Reader(reader), status_flag)
            }
            AccessMode::O_WRONLY => {
                let writer = pipe.get_or_create_writer();

                self.wait_queue.wake_all();

                if !pipe.has_read_handle() {
                    if status_flag.contains(StatusFlags::O_NONBLOCK) {
                        return_errno_with_message!(Errno::ENXIO, "no reader is present");
                    }

                    let old_read_count = pipe.read_count;
                    drop(pipe);
                    self.wait_queue.pause_until(|| {
                        (old_read_count != self.pipe.lock().read_count).then_some(())
                    })?;
                }

                NamedPipeHandle::new(HandleInner::Writer(writer), status_flag)
            }
            AccessMode::O_RDWR => {
                let (reader, writer) = pipe.get_or_create_reader_writer();
                self.wait_queue.wake_all();
                NamedPipeHandle::new(HandleInner::ReaderWriter(reader, writer), status_flag)
            }
        };
        Ok(handle)
    }
}

#[derive(Default)]
struct NamedPipeInner {
    read_handle: Weak<ReadHandle>,
    write_handle: Weak<WriteHandle>,
    read_count: usize,
    write_count: usize,
}

impl NamedPipeInner {
    fn has_read_handle(&self) -> bool {
        self.read_handle.strong_count() > 0
    }

    fn has_write_handle(&self) -> bool {
        self.write_handle.strong_count() > 0
    }

    fn get_or_create_reader(&mut self) -> Arc<ReadHandle> {
        self.read_count += 1;

        if let Some(reader) = self.read_handle.upgrade() {
            return reader;
        }

        if let Some(writer) = self.write_handle.upgrade() {
            let reader = Arc::new(ReadHandle {
                inner: writer.inner.clone(),
            });
            self.read_handle = Arc::downgrade(&reader);
            reader.inner.0.state.peer_activate();
            return reader;
        }

        let (reader, writer) = pipe::new_pair();
        let read_handle = Arc::new(ReadHandle {
            inner: Arc::new((reader, writer)),
        });
        self.read_handle = Arc::downgrade(&read_handle);

        read_handle
    }

    fn get_or_create_writer(&mut self) -> Arc<WriteHandle> {
        self.write_count += 1;

        if let Some(writer) = self.write_handle.upgrade() {
            return writer;
        }

        if let Some(reader) = self.read_handle.upgrade() {
            let writer = Arc::new(WriteHandle {
                inner: reader.inner.clone(),
            });
            self.write_handle = Arc::downgrade(&writer);
            writer.inner.1.state.activate();
            return writer;
        }

        let (reader, writer) = pipe::new_pair();
        let write_handle = Arc::new(WriteHandle {
            inner: Arc::new((reader, writer)),
        });
        self.write_handle = Arc::downgrade(&write_handle);

        write_handle
    }

    fn get_or_create_reader_writer(&mut self) -> (Arc<ReadHandle>, Arc<WriteHandle>) {
        self.read_count += 1;
        self.write_count += 1;

        let reader = self.read_handle.upgrade();
        let writer = self.write_handle.upgrade();
        match (reader, writer) {
            (Some(reader), Some(writer)) => (reader, writer),
            (Some(reader), None) => {
                let writer = Arc::new(WriteHandle {
                    inner: reader.inner.clone(),
                });
                self.write_handle = Arc::downgrade(&writer);
                writer.inner.1.state.activate();

                (reader, writer)
            }
            (None, Some(writer)) => {
                let reader = Arc::new(ReadHandle {
                    inner: writer.inner.clone(),
                });
                self.read_handle = Arc::downgrade(&reader);
                reader.inner.0.state.peer_activate();

                (reader, writer)
            }
            (None, None) => {
                let (reader, writer) = pipe::new_pair();
                let read_handle = Arc::new(ReadHandle {
                    inner: Arc::new((reader, writer)),
                });
                let write_handle = Arc::new(WriteHandle {
                    inner: read_handle.inner.clone(),
                });
                self.read_handle = Arc::downgrade(&read_handle);
                self.write_handle = Arc::downgrade(&write_handle);

                (read_handle, write_handle)
            }
        }
    }
}
