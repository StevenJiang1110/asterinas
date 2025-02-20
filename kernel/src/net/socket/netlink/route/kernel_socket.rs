// SPDX-License-Identifier: MPL-2.0

use core::marker::PhantomData;

use crate::{net::socket::netlink::route::message::{CLinkMessage, GetRequest}, prelude::*};

use super::message::{self, AnyRequestMessage};

pub struct NetlinkRouteKernelSocket {
    _private: PhantomData<()>,
}

impl NetlinkRouteKernelSocket {
    const fn new() -> Self {
        Self {
            _private: PhantomData,
        }
    }

    fn request<F: FnOnce()>(&self, message: &dyn AnyRequestMessage, response: F) -> Result<()> {
        if let Some(get_link_request) = message.as_any().downcast_ref::<GetRequest<CLinkMessage>>() {
            do_get_link(get_link_request);
        }

        todo!()
    }
}

/// FIXME: NETLINK_ROUTE_KERNEL should be a per net namespace socket
static NETLINK_ROUTE_KERNEL: NetlinkRouteKernelSocket = NetlinkRouteKernelSocket::new();

fn do_get_link(request: &GetRequest<CLinkMessage>) {
    let message = request.message();
    
    if message.index != 0 {
        todo!("find the specific device")
    }

    // Find all 
}
