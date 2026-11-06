// SPDX-License-Identifier: MPL-2.0

use super::legacy::CRtGenMsg;
use crate::{
    net::socket::netlink::{
        message::{SegmentBody, SegmentCommon},
        route::message::attr::route::RouteAttr,
    },
    prelude::*,
};

pub(in crate::net::socket::netlink::route) type RouteSegment = SegmentCommon<CRtMsg, RouteAttr>;

impl SegmentBody for CRtMsg {
    type CLegacyType = CRtGenMsg;
}

/// `rtmsg` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/rtnetlink.h#L237>.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct CRtMsg {
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

/// Flags in [`CRtMsg`].
#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
pub(crate) struct RouteMessageFlags(u32);

bitflags! {
    impl RouteMessageFlags: u32 {
        const NOTIFY       = 0x100;
        const CLONED       = 0x200;
        const PREFIX       = 0x800;
        const LOOKUP_TABLE = 0x1000;
        const FIB_MATCH    = 0x2000;

        // IPv4 route-cache flags share the rtmsg flags field.
        const IPV4_NOTIFY    = 0x00010000;
        const IPV4_LOCAL     = 0x80000000;
        const IPV4_BROADCAST = 0x10000000;
        const IPV4_MULTICAST = 0x20000000;

        const _ = !0;
    }
}

/// The protocol that installed a route.
#[repr(u8)]
#[derive(Clone, Copy, Debug)]
pub(crate) enum RouteProtocol {
    Kernel = 2,
    Boot = 3,
}
