// SPDX-License-Identifier: MPL-2.0

//! Netlink Route Socket.

use core::sync::atomic::{AtomicBool, Ordering};

use bound::BoundNetlinkRoute;
use unbound::UnboundNetlinkRoute;

use super::AnyNetlinkSocket;
use crate::{
    events::IoEvents,
    net::socket::{private::SocketPrivate, MessageHeader, SendRecvFlags, Socket},
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
mod unbound;

pub struct NetlinkRouteSocket {
    is_nonblocking: AtomicBool,
    pollee: Pollee,
    inner: Inner,
}

enum Inner {
    Unbound(UnboundNetlinkRoute),
    Bound(BoundNetlinkRoute),
}

impl NetlinkRouteSocket {
    pub fn new(is_nonblocking: bool) -> Self {
        Self {
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
            inner: Inner::Unbound(UnboundNetlinkRoute::new()),
        }
    }

    fn check_io_events(&self) -> IoEvents {
        todo!()
    }
}

impl Socket for NetlinkRouteSocket {
    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        todo!()
    }

    fn recvmsg(
        &self,
        writers: &mut dyn MultiWrite,
        flags: SendRecvFlags,
    ) -> Result<(usize, MessageHeader)> {
        todo!()
    }
}

impl SocketPrivate for NetlinkRouteSocket {
    fn is_nonblocking(&self) -> bool {
        self.is_nonblocking.load(Ordering::Relaxed)
    }

    fn set_nonblocking(&self, nonblocking: bool) {
        self.is_nonblocking.store(nonblocking, Ordering::Relaxed);
    }
}

impl Pollable for NetlinkRouteSocket {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.pollee
            .poll_with(mask, poller, || self.check_io_events())
    }
}

impl AnyNetlinkSocket for NetlinkRouteSocket {}
