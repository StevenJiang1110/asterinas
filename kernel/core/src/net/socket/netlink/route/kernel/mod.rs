// SPDX-License-Identifier: MPL-2.0

//! This module defines the kernel socket,
//! which is responsible for handling requests from user space.

use core::marker::PhantomData;

use super::message::{RtnlMessage, RtnlSegment};
use crate::{
    net::socket::netlink::{
        addr::PortNum,
        message::{ErrorSegment, ProtocolSegment, SegHdrCommonFlags},
        table::{NetlinkRouteProtocol, SupportedNetlinkProtocol},
    },
    prelude::*,
};

mod addr;
mod link;
mod route;
mod util;

pub(super) struct NetlinkRouteKernelSocket {
    _private: PhantomData<()>,
}

impl NetlinkRouteKernelSocket {
    const fn new() -> Self {
        Self {
            _private: PhantomData,
        }
    }

    pub(super) fn handle_request(
        &self,
        request: &RtnlSegment,
        dst_port: PortNum,
        strict_check: bool,
    ) {
        debug!("netlink route request: {:?}", request);

        let request_header = request.header();

        let response_segments = match request {
            RtnlSegment::GetLink(request_segment) => {
                link::do_get_link(request_segment, strict_check)
            }
            RtnlSegment::GetAddr(request_segment) => {
                addr::do_get_addr(request_segment, strict_check)
            }
            RtnlSegment::GetRoute(request_segment) => {
                route::do_get_route(request_segment, strict_check)
            }
            _ => Err(Error::with_message(
                Errno::EOPNOTSUPP,
                "the netlink route request is not supported",
            )),
        };

        let mut response_segments = match response_segments {
            Ok(segments) => segments,
            Err(error) => {
                // Errors are reported regardless of the ACK flag.
                // Reference: <https://docs.kernel.org/userspace-api/netlink/intro.html#netlink-message-types>.
                let err_segment = ErrorSegment::new_from_request(request_header, Some(error));
                self.report_error(err_segment, dst_port);
                return;
            }
        };

        // Modern Linux may place the `NLMSG_DONE` segment
        // in the same message as the preceding segments,
        // depending on the message type.
        // Legacy Linux generally delivered the `NLMSG_DONE` segment
        // in a separate message.
        // We follow the legacy Linux behavior here because it is simpler.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/core/rtnetlink.c#L6870>.
        let done_segments = if matches!(response_segments.last(), Some(RtnlSegment::Done(_))) {
            response_segments.split_off(response_segments.len() - 1)
        } else {
            Vec::new()
        };

        let needs_ack = done_segments.is_empty()
            && SegHdrCommonFlags::from_bits_truncate(request_header.flags)
                .contains(SegHdrCommonFlags::ACK);
        for segments in [response_segments, done_segments] {
            if segments.is_empty() {
                continue;
            }

            let response = RtnlMessage::new(segments);
            debug!("netlink route response: {:?}", response);

            NetlinkRouteProtocol::unicast(dst_port, response).unwrap();
        }

        if needs_ack {
            self.report_error(
                ErrorSegment::new_from_request(request_header, None),
                dst_port,
            );
        }
    }

    // FIXME: This method only accepts `ErrorSegment`,
    // but some errors should be reported using `DoneSegment` with a negative errno.
    // For example, Linux reports validation failures in strict dump requests this way.
    // Adjust this API and its callers to support both response types.
    pub(super) fn report_error(&self, err_segment: ErrorSegment, dst_port: PortNum) {
        let response = RtnlMessage::new(vec![RtnlSegment::Error(err_segment)]);

        debug!("netlink route error: {:?}", response);

        NetlinkRouteProtocol::unicast(dst_port, response).unwrap();
    }
}

/// FIXME: NETLINK_ROUTE_KERNEL should be a per-network namespace socket
static NETLINK_ROUTE_KERNEL: NetlinkRouteKernelSocket = NetlinkRouteKernelSocket::new();

pub(super) fn get_netlink_route_kernel() -> &'static NetlinkRouteKernelSocket {
    &NETLINK_ROUTE_KERNEL
}
