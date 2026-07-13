// SPDX-License-Identifier: MPL-2.0

use super::legacy::CRtGenMsg;
use crate::{
    net::{
        route::{RouteFlags, RouteProtocol, RouteScope, RouteTableId, RouteType},
        socket::netlink::{
            message::{SegmentBody, SegmentCommon},
            route::message::attr::route::RouteAttr,
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub type RouteSegment = SegmentCommon<RouteSegmentBody, RouteAttr>;

impl SegmentBody for RouteSegmentBody {
    type CLegacyType = CRtGenMsg;
    type CType = CRtMsg;
}

/// `rtmsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/include/uapi/linux/rtnetlink.h#L237>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub struct CRtMsg {
    pub family: u8,
    pub dst_len: u8,
    pub src_len: u8,
    pub tos: u8,
    pub table: u8,
    pub protocol: u8,
    pub scope: u8,
    pub type_: u8,
    pub flags: u32,
}

#[derive(Clone, Copy, Debug)]
pub struct RouteSegmentBody {
    pub family: i32,
    pub dst_len: u8,
    pub src_len: u8,
    pub tos: u8,
    pub table: Option<RouteTableId>,
    pub protocol: RouteProtocol,
    pub scope: RouteScope,
    pub type_: RouteType,
    pub flags: RouteFlags,
}

impl TryFrom<CRtMsg> for RouteSegmentBody {
    type Error = Error;

    fn try_from(value: CRtMsg) -> Result<Self> {
        let family = value.family as i32;
        if family != CSocketAddrFamily::AF_INET as i32
            && family != CSocketAddrFamily::AF_UNSPEC as i32
        {
            return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "only IPv4 route requests are supported"
            );
        }
        if value.dst_len > 32 || value.src_len > 32 {
            return_errno_with_message!(Errno::EINVAL, "the route prefix length is invalid");
        }

        Ok(Self {
            family,
            dst_len: value.dst_len,
            src_len: value.src_len,
            tos: value.tos,
            table: RouteTableId::from_rtmsg_table(value.table),
            protocol: RouteProtocol::new(value.protocol),
            scope: RouteScope::new(value.scope),
            type_: RouteType::new(value.type_),
            flags: RouteFlags::from_bits(value.flags).ok_or_else(|| {
                Error::with_message(Errno::EOPNOTSUPP, "the route flags are not supported")
            })?,
        })
    }
}

impl From<RouteSegmentBody> for CRtMsg {
    fn from(value: RouteSegmentBody) -> Self {
        Self {
            family: value.family as u8,
            dst_len: value.dst_len,
            src_len: value.src_len,
            tos: value.tos,
            table: value
                .table
                .map(RouteTableId::rtmsg_table)
                .unwrap_or(RouteTableId::UNSPEC.rtmsg_table()),
            protocol: value.protocol.get(),
            scope: value.scope.get(),
            type_: value.type_.get(),
            flags: value.flags.bits(),
        }
    }
}

impl From<CRtGenMsg> for CRtMsg {
    fn from(value: CRtGenMsg) -> Self {
        Self {
            family: value.family,
            dst_len: 0,
            src_len: 0,
            tos: 0,
            table: 0,
            protocol: 0,
            scope: 0,
            type_: RouteType::UNSPEC.get(),
            flags: 0,
        }
    }
}
