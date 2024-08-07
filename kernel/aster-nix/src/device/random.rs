// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use crate::{
    events::IoEvents,
    fs::{
        device::{Device, DeviceId, DeviceType},
        inode_handle::FileIo,
    },
    prelude::*,
    process::signal::Poller,
    util::random::getrandom,
};

pub struct Random;

impl Random {
    pub fn getrandom(buf: &mut [u8]) -> Result<usize> {
        getrandom(buf)?;
        Ok(buf.len())
    }
}

impl Device for Random {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        // The same value as Linux
        DeviceId::new(1, 8)
    }
}

impl FileIo for Random {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        Self::getrandom(buf)
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        Ok(buf.len())
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut Poller>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
    }
}
