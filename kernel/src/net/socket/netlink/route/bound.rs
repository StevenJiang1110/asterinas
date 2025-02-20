// SPDX-License-Identifier: MPL-2.0

use crate::net::socket::netlink::table::BoundHandle;

pub struct BoundNetlinkRoute {
    handle: BoundHandle,
}

impl BoundNetlinkRoute {
    pub const fn new(handle: BoundHandle) -> Self {
        Self { handle }
    }
}
