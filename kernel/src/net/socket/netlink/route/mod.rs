// SPDX-License-Identifier: MPL-2.0

//! Netlink Route Socket.

use core::sync::atomic::{AtomicBool, Ordering};

use bound::BoundNetlinkRoute;
use message::NetlinkMessageHeader;
use ostd::early_println;
use takeable::Takeable;
use unbound::UnboundNetlinkRoute;

use super::{AnyNetlinkSocket, NetlinkSocketAddr};
use crate::{
    events::IoEvents,
    net::socket::{
        netlink::message::CNetlinkMessageHeader, private::SocketPrivate, MessageHeader,
        SendRecvFlags, Socket, SocketAddr,
    },
    prelude::*,
    process::signal::{PollHandle, Pollable, Pollee},
    util::{MultiRead, MultiWrite},
};

mod bound;
mod message;
mod unbound;

pub struct NetlinkRouteSocket {
    is_nonblocking: AtomicBool,
    pollee: Pollee,
    inner: RwMutex<Takeable<Inner>>,
    weak_self: Weak<dyn AnyNetlinkSocket>,
}

enum Inner {
    Unbound(UnboundNetlinkRoute),
    Bound(BoundNetlinkRoute),
}

impl NetlinkRouteSocket {
    pub fn new(is_nonblocking: bool) -> Arc<Self> {
        Arc::new_cyclic(|weak_self| Self {
            is_nonblocking: AtomicBool::new(is_nonblocking),
            pollee: Pollee::new(),
            inner: RwMutex::new(Takeable::new(Inner::Unbound(UnboundNetlinkRoute::new()))),
            weak_self: weak_self.clone() as _,
        })
    }

    fn check_io_events(&self) -> IoEvents {
        todo!()
    }
}

impl Socket for NetlinkRouteSocket {
    fn bind(&self, socket_addr: SocketAddr) -> Result<()> {
        let SocketAddr::Netlink(netlink_addr) = socket_addr else {
            return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the provided address is not netlink address"
            );
        };

        let mut inner = self.inner.write();
        inner.borrow_result(
            |owned_inner| match owned_inner.bind(&netlink_addr, &self.weak_self) {
                Ok(bound_inner) => (bound_inner, Ok(())),
                Err((err, err_inner)) => (err_inner, Err(err)),
            },
        )
    }

    fn addr(&self) -> Result<SocketAddr> {
        let netlink_addr = match self.inner.read().as_ref() {
            Inner::Unbound(_) => NetlinkSocketAddr::new_unspecified(),
            Inner::Bound(bound) => bound.addr(),
        };

        Ok(SocketAddr::Netlink(netlink_addr))
    }

    fn sendmsg(
        &self,
        reader: &mut dyn MultiRead,
        message_header: MessageHeader,
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let netlink_message_header = {
            let c_netlink_message_header = reader.read_val::<CNetlinkMessageHeader>()?;
            early_println!("header = {:?}", c_netlink_message_header);
            NetlinkMessageHeader::try_from(c_netlink_message_header)?
        };
        early_println!("header = {:?}", netlink_message_header);
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

impl Inner {
    fn bind(
        self,
        addr: &NetlinkSocketAddr,
        socket: &Weak<dyn AnyNetlinkSocket>,
    ) -> core::result::Result<Self, (Error, Self)> {
        let unbound = match self {
            Inner::Unbound(unbound) => unbound,
            Inner::Bound(bound) => {
                // FIXME: We need to further check the Linux behavior
                // whether we should return error if the socket is bound.
                // The socket may call `bind` syscall to join new multicast groups.
                return Err((
                    Error::with_message(Errno::EINVAL, "the socket is already bound"),
                    Self::Bound(bound),
                ));
            }
        };

        match unbound.bind(addr, socket) {
            Ok(bound) => Ok(Self::Bound(bound)),
            Err((err, unbound)) => Err((err, Self::Unbound(unbound))),
        }
    }
}
