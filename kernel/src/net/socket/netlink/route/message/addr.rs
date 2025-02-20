// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

/// Corresponding to `ifaddrmsg` in Linux
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct CAddrMessage {
    family: u8,
    /// The prefix length
    prefix_len: u8,
    /// Flags
    flags: u8,
    /// Address scope
    scope: u8,
    /// Link index
    index: u32,
}