// SPDX-License-Identifier: MPL-2.0

//! The netlink message types for netlink route protocol.

use crate::{
    net::socket::netlink::{
        addr::PortNum,
        message::{
            CNetlinkMessageHeader, DeleteRequestFlags, GetRequestFlags, NetlinkMessageCommonFlags,
            NewRequestFlags,
        },
    },
    prelude::*,
};

#[derive(Debug)]
pub struct NetlinkMessageHeader {
    op_and_flags: OpAndFlags,
    common_flags: NetlinkMessageCommonFlags,
    content_type: MessageConentType,
    sequence_number: u32,
    sender_port_id: PortNum,
}

impl TryFrom<CNetlinkMessageHeader> for NetlinkMessageHeader {
    type Error = Error;

    fn try_from(value: CNetlinkMessageHeader) -> Result<Self> {
        let message_type = CMessageType::try_from(value.type_)?;

        let op_and_flags = if message_type.is_new_ruquest() {
            OpAndFlags::New(NewRequestFlags::from_bits_truncate(value.flags))
        } else if message_type.is_del_request() {
            OpAndFlags::Delete(DeleteRequestFlags::from_bits_truncate(value.flags))
        } else if message_type.is_get_request() {
            OpAndFlags::Get(GetRequestFlags::from_bits_truncate(value.flags))
        } else {
            OpAndFlags::Set
        };

        let common_flags = NetlinkMessageCommonFlags::from_bits_truncate(value.flags);
        let content_type = message_type.content_type();

        Ok(Self {
            op_and_flags,
            common_flags,
            content_type,
            sequence_number: value.seq,
            sender_port_id: value.pid,
        })
    }
}

#[derive(Debug)]
pub enum OpAndFlags {
    New(NewRequestFlags),
    Delete(DeleteRequestFlags),
    Get(GetRequestFlags),
    Set,
}

#[derive(Debug)]
pub enum MessageConentType {
    Link,
    Addr,
    Route,
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
pub enum CMessageType {
    NEWLINK = 16,
    DELLINK = 17,
    GETLINK = 18,
    SETLINK = 19,

    NEWADDR = 20,
    DELADDR = 21,
    GETADDR = 22,

    NEWROUTE = 24,
    DELROUTE = 25,
    GETROUTE = 26,
    // TODO: The list is not exhaustive now.
}

impl CMessageType {
    const fn is_new_ruquest(&self) -> bool {
        (*self as u16) & 0x3 == 0x0
    }

    const fn is_del_request(&self) -> bool {
        (*self as u16) & 0x3 == 0x1
    }

    const fn is_get_request(&self) -> bool {
        (*self as u16) & 0x3 == 0x2
    }

    const fn content_type(&self) -> MessageConentType {
        match self {
            Self::NEWLINK | Self::DELLINK | Self::GETLINK | Self::SETLINK => {
                MessageConentType::Link
            }
            Self::NEWADDR | Self::DELADDR | Self::GETADDR => MessageConentType::Addr,
            Self::NEWROUTE | Self::DELROUTE | Self::GETROUTE => MessageConentType::Route,
        }
    }
}
