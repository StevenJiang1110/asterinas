// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::{sync::WaitQueue, task::Task};

use super::{
    file_handle::FileLike,
    pipe::{self, PipeReader, PipeWriter},
    utils::{AccessMode, Metadata},
};
use crate::{
    events::IoEvents,
    fs::{fs_resolver::OpenArgs, utils::StatusFlags},
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct NamedPipe {
    reader: Arc<PipeReader>,
    writer: Arc<PipeWriter>,
    has_reader: AtomicBool,
    has_writer: AtomicBool,
    open_wait_queue: WaitQueue,
}

impl NamedPipe {
    pub fn new() -> Result<Self> {
        let (reader, writer) = pipe::new_pair()?;

        Ok(Self {
            reader,
            writer,
            has_reader: AtomicBool::new(false),
            has_writer: AtomicBool::new(false),
            open_wait_queue: WaitQueue::new(),
        })
    }

    pub fn with_capacity(capacity: usize) -> Result<Self> {
        let (reader, writer) = pipe::new_pair_with_capacity(capacity)?;

        Ok(Self {
            reader,
            writer,
            has_reader: AtomicBool::new(false),
            has_writer: AtomicBool::new(false),
            open_wait_queue: WaitQueue::new(),
        })
    }

    pub fn open(&self, open_args: OpenArgs) {
        if open_args.status_flags.contains(StatusFlags::O_PATH) {
            return;
        }

        let access_mode = open_args.access_mode;
        let is_nonblocking = open_args.status_flags.contains(StatusFlags::O_NONBLOCK);
        if access_mode == AccessMode::O_RDONLY {
            self.has_reader.store(true, Ordering::Release);
            self.open_wait_queue.wake_all();

            if self.has_writer.load(Ordering::Acquire) {
                return;
            }

            if is_nonblocking {
                // self.reader.set_status_flags(StatusFlags::O_NONBLOCK).unwrap();
                return;
            }
            // else {
            //     self.reader.set_status_flags(StatusFlags::empty()).unwrap()
            // }

            self.open_wait_queue
                .pause_until(|| self.has_writer.load(Ordering::Acquire).then_some(()))
                .unwrap();
        }

        if access_mode == AccessMode::O_WRONLY {
            self.has_writer.store(true, Ordering::Release);
            self.open_wait_queue.wake_all();

            if self.has_reader.load(Ordering::Acquire) {
                return;
            }

            if is_nonblocking {
                todo!("return ENXIO");
            }

            self.open_wait_queue
                .pause_until(|| self.has_reader.load(Ordering::Acquire).then_some(()))
                .unwrap();
        }

        if access_mode == AccessMode::O_RDWR {
            unimplemented!()
        }
    }
}

impl Pollable for NamedPipe {
    fn poll(&self, _mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        warn!("Named pipe doesn't support poll now, return IoEvents::empty for now.");
        IoEvents::empty()
    }
}

impl FileLike for NamedPipe {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        println!("read named pipe");
        let res = self.reader.read(writer);
        println!("read named pipe returns");
        res
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        // println!("write to named pipe: {}", reader.remain());
        self.writer.write(reader)
    }

    fn access_mode(&self) -> AccessMode {
        AccessMode::O_RDWR
    }

    fn metadata(&self) -> Metadata {
        self.reader.metadata()
    }
}
