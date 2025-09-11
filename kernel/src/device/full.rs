// SPDX-License-Identifier: MPL-2.0

use super::*;
use crate::{
    events::IoEvents,
    fs::inode_handle::FileIo,
    prelude::*,
    process::signal::{PollHandle, Pollable},
};

pub struct Full;

impl Device for Full {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // Same value with Linux
        DeviceId::new(1, 7)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        Ok(Some(Arc::new(Full)))
    }
}

impl Pollable for Full {
    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}

impl FileIo for Full {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        let len = writer.avail();
        writer.fill_zeros(len)?;
        Ok(len)
    }

    fn write(&self, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::ENOSPC, "trying to write to /dev/full")
    }
}
