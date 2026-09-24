// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(netlink)]

use core::num::NonZeroU32;

use aster_bigtcp::iface::{InterfaceFlags, InterfaceType};

use super::legacy::CRtGenMsg;
use crate::{
    net::socket::netlink::{
        message::{SegmentBody, SegmentCommon},
        route::message::attr::link::LinkAttr,
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(in netlink) type LinkSegment = SegmentCommon<LinkSegmentBody, LinkAttr>;

impl SegmentBody for LinkSegmentBody {
    type CLegacyType = CRtGenMsg;
    type CType = CIfinfoMsg;

    fn validate_c_type(value: &Self::CType, strict_check: bool) -> Result<()> {
        if strict_check
            && (value._pad != 0 || value.type_ != 0 || value.flags != 0 || value.change != 0)
        {
            return_errno_with_message!(Errno::EINVAL, "the link request header is not valid");
        }
        Ok(())
    }
}

/// `ifinfomsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L561>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CIfinfoMsg {
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

#[derive(Clone, Copy, Debug)]
pub(crate) struct LinkSegmentBody {
    pub family: CSocketAddrFamily,
    pub type_: InterfaceType,
    pub index: Option<NonZeroU32>,
    pub flags: InterfaceFlags,
}

impl TryFrom<CIfinfoMsg> for LinkSegmentBody {
    type Error = Error;

    fn try_from(value: CIfinfoMsg) -> Result<Self> {
        let family = CSocketAddrFamily::try_from(value.family as i32)
            .unwrap_or(CSocketAddrFamily::AF_UNSPEC);
        let type_ = InterfaceType::try_from(value.type_).unwrap_or(InterfaceType::NETROM);
        let index = NonZeroU32::new(value.index);
        let flags = InterfaceFlags::from_bits_truncate(value.flags);

        Ok(Self {
            family,
            type_,
            index,
            flags,
        })
    }
}

impl From<LinkSegmentBody> for CIfinfoMsg {
    fn from(value: LinkSegmentBody) -> Self {
        CIfinfoMsg {
            family: value.family as _,
            _pad: 0,
            type_: value.type_ as _,
            index: value.index.map(NonZeroU32::get).unwrap_or(0),
            flags: value.flags.bits(),
            change: 0,
        }
    }
}
