// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

use super::{AnyRequestMessage, GetRequest};

/// Link level specific information, corresponding to `ifinfomsg` in Linux
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CLinkMessage {
    /// AF_UNSPEC
    pub family: u8,
    /// Padding byte
    pub _pad: u8,
    /// Device type
    pub type_: u16,
    /// Interface index
    pub index: u32,
    /// Device flags
    pub flags: u32,
    /// Change mask
    pub change: u32,
}
