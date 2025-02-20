// SPDX-License-Identifier: MPL-2.0

//! The netlink message types for netlink route protocol.

mod link;
mod addr;
mod route;

use crate::{net::socket::netlink::message::{GetRequestFlags, NetlinkMessageCommonFlags}, prelude::*};

pub use link::CLinkMessage;

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
}

pub trait AnyRequestMessage: Send + Sync + Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

pub trait AnyResponseMessage: Send + Sync + Any {
    fn as_any(&self) -> &dyn Any;
    fn as_any_mut(&mut self) -> &mut dyn Any;
}

#[derive(Debug)]
pub struct GetRequest<M: Debug> { 
    message: M,
    flags: GetRequestFlags,
}

impl <M: Pod + Debug> GetRequest<M> {
    pub fn new(message: M, flags: u16) -> Self {
        let common_flags = NetlinkMessageCommonFlags::from_bits_truncate(flags);
        debug_assert_eq!(common_flags, NetlinkMessageCommonFlags::REQUEST);

        let flags = GetRequestFlags::from_bits_truncate(flags);

        Self { message, flags }
    }

    pub const fn message(&self) -> &M {
        &self.message
    }

    pub const fn flags(&self) -> GetRequestFlags {
        self.flags
    }
}

macro_rules! impl_any_request_message {
    ($message_type:ty) => {
        impl AnyRequestMessage for $message_type {
            fn as_any(&self) -> &dyn Any {
                self
            }

            fn as_any_mut(&mut self) -> &mut dyn Any {
                self
            }
        }
    };
}

impl_any_request_message!(GetRequest<CLinkMessage>);
