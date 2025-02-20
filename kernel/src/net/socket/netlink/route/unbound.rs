// SPDX-License-Identifier: MPL-2.0

use super::bound::BoundNetlinkRoute;
use crate::{
    net::socket::netlink::{
        table::NETLINK_SOCKET_TABLE, AnyNetlinkSocket, NetlinkSocketAddr, StandardNetlinkProtocol,
    },
    prelude::*,
};

pub struct UnboundNetlinkRoute {
    _private: (),
}

impl UnboundNetlinkRoute {
    pub const fn new() -> Self {
        Self { _private: () }
    }

    pub fn bind(
        self,
        addr: &NetlinkSocketAddr,
        socket: &Weak<dyn AnyNetlinkSocket>,
    ) -> core::result::Result<BoundNetlinkRoute, (Error, Self)> {
        let bound_handle = NETLINK_SOCKET_TABLE
            .bind(StandardNetlinkProtocol::NETLINK_ROUTE as _, addr, socket)
            .map_err(|err| (err, self))?;

        Ok(BoundNetlinkRoute::new(bound_handle))
    }
}
