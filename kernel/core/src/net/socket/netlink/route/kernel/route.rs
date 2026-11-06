// SPDX-License-Identifier: MPL-2.0

//! Handle route-related requests.

use core::net::IpAddr;

use aster_bigtcp::wire::IpCidr;

use super::util::{self, ResponseMode};
use crate::{
    net::{
        iface,
        route::{self, RouteAddressFamily, RouteEntry, RouteType},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{
                CRtMsg, RouteAttr, RouteAttrClass, RouteMessageFlags, RouteProtocol, RouteSegment,
                RtScope, RtnlSegment,
            },
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

pub(super) fn do_get_route(
    request_segment: &RouteSegment,
    strict_check: bool,
) -> Result<Vec<RtnlSegment>> {
    let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
    if !flags.intersects(GetRequestFlags::DUMP) {
        return_errno_with_message!(Errno::EOPNOTSUPP, "single-route queries are not supported");
    }
    validate_getroute_request(request_segment, strict_check)?;

    let filter_by = FilterBy::from_request(request_segment, strict_check)?;
    let requested_family = request_segment.body().family;
    let dump_ipv4 = requested_family != CSocketAddrFamily::AF_INET6 as u8;
    let dump_ipv6 = requested_family != CSocketAddrFamily::AF_INET as u8;

    let ipv4_segments = route::ipv4_routes()
        .filter(|entry| dump_ipv4 && filter_by.matches(entry))
        .map(|entry| RouteInfo::from_entry(entry).into_segment(request_segment.header()));
    let ipv6_segments = route::ipv6_routes()
        .filter(|entry| dump_ipv6 && filter_by.matches(entry))
        .map(|entry| RouteInfo::from_entry(entry).into_segment(request_segment.header()));
    let mut response_segments = ipv4_segments
        .chain(ipv6_segments)
        .map(RtnlSegment::NewRoute)
        .collect();

    let response_mode = if filter_by.is_filtered {
        ResponseMode::FilteredDump
    } else {
        ResponseMode::Dump
    };

    util::finish_response(
        request_segment.header(),
        response_mode,
        &mut response_segments,
    );

    Ok(response_segments)
}

struct FilterBy {
    table: u32,
    protocol: u8,
    type_: u8,
    output_index: Option<u32>,
    flags: RouteMessageFlags,
    is_filtered: bool,
}

impl FilterBy {
    fn from_request(request_segment: &RouteSegment, strict_check: bool) -> Result<Self> {
        let body = request_segment.body();
        let flags = RouteMessageFlags::from_bits_truncate(body.flags)
            & (RouteMessageFlags::CLONED | RouteMessageFlags::PREFIX);
        let mut filter_by = Self {
            table: 0,
            protocol: 0,
            type_: 0,
            output_index: None,
            flags,
            is_filtered: false,
        };
        if !strict_check {
            return Ok(filter_by);
        }

        filter_by.table = body.table as u32;
        filter_by.protocol = body.protocol;
        filter_by.type_ = body.type_;
        for attr in request_segment.attrs() {
            match attr {
                RouteAttr::Table(table) => filter_by.table = *table,
                RouteAttr::Oif(index) => filter_by.output_index = Some(*index),
                _ => {}
            }
        }

        if let Some(index) = filter_by.output_index
            && !iface::iter_all_ifaces().any(|iface| iface.index() == index)
        {
            return_errno_with_message!(Errno::ENODEV, "the interface does not exist");
        }

        // An explicitly selected missing table fails for a single address family.
        // An AF_UNSPEC dump skips tables absent from one of the families.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/fib_frontend.c#L1048>.
        let table_exists = match CSocketAddrFamily::try_from(body.family as i32) {
            Ok(CSocketAddrFamily::AF_INET) => route::has_ipv4_table(filter_by.table),
            Ok(CSocketAddrFamily::AF_INET6) => route::has_ipv6_table(filter_by.table),
            _ => true,
        };
        if filter_by.table != 0 && !table_exists {
            return_errno_with_message!(Errno::ENOENT, "the route table does not exist");
        }

        filter_by.is_filtered = filter_by.table != 0
            || filter_by.protocol != 0
            || filter_by.type_ != 0
            || filter_by.output_index.is_some()
            || !filter_by.flags.is_empty();
        Ok(filter_by)
    }

    fn matches<A: RouteAddressFamily>(&self, entry: &RouteEntry<A>) -> bool {
        // We have neither cached route exceptions nor IPv6 prefix routes.
        self.flags.is_empty()
            && (self.table == 0 || self.table == entry.table_id().get())
            && (self.protocol == 0 || self.protocol == route_protocol(entry) as u8)
            && (self.type_ == 0 || self.type_ == entry.type_() as u8)
            && self
                .output_index
                .is_none_or(|index| index == entry.output_iface().index())
    }
}

fn validate_getroute_request(request_segment: &RouteSegment, strict_check: bool) -> Result<()> {
    if !strict_check {
        return Ok(());
    }
    let body = request_segment.body();
    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/fib_frontend.c#L940>.
    if body.dst_len != 0 || body.src_len != 0 || body.tos != 0 || body.scope != 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "the route dump header contains nonzero prefix length, TOS, or scope"
        );
    }
    let allowed_flags = RouteMessageFlags::CLONED | RouteMessageFlags::PREFIX;
    if body.flags & !allowed_flags.bits() != 0 {
        return_errno_with_message!(Errno::EINVAL, "the route dump flags are invalid");
    }
    let allowed = [RouteAttrClass::TABLE, RouteAttrClass::OIF];
    for attr in request_segment.attrs() {
        if !allowed.contains(&attr.class()) {
            return_errno_with_message!(
                Errno::EINVAL,
                "the attribute is not allowed in this route request"
            );
        }
    }
    Ok(())
}

/// Route properties ready to report, independent of a stored FIB entry.
struct RouteInfo {
    dst: IpAddr,
    dst_len: u8,
    source: Option<IpAddr>,
    gateway: Option<IpAddr>,
    output_index: u32,
    table: u32,
    metric: u32,
    protocol: RouteProtocol,
    scope: RtScope,
    type_: RouteType,
    flags: RouteMessageFlags,
}

impl RouteInfo {
    fn from_entry<A: RouteAddressFamily>(entry: &RouteEntry<A>) -> Self {
        let (dst, dst_len, source) = match (*entry.dst()).into() {
            IpCidr::Ipv4(cidr) => (
                IpAddr::from(cidr.address()),
                cidr.prefix_len(),
                entry
                    .output_iface()
                    .ipv4_cidr()
                    .map(|cidr| cidr.address().into()),
            ),
            IpCidr::Ipv6(cidr) => (IpAddr::from(cidr.address()), cidr.prefix_len(), None),
        };
        let scope = if dst.is_ipv6() || entry.gateway().is_some() {
            RtScope::UNIVERSE
        } else if entry.type_() == RouteType::Local {
            RtScope::HOST
        } else {
            RtScope::LINK
        };
        Self {
            dst,
            dst_len,
            source,
            gateway: entry.gateway().map(Into::into),
            output_index: entry.output_iface().index(),
            table: entry.table_id().get(),
            metric: entry.metric().get(),
            protocol: route_protocol(entry),
            scope,
            type_: entry.type_(),
            flags: RouteMessageFlags::empty(),
        }
    }

    fn into_segment(self, request_header: &CMsgSegHdr) -> RouteSegment {
        let header = CMsgSegHdr {
            len: 0,
            type_: CSegmentType::NEWROUTE as _,
            flags: SegHdrCommonFlags::empty().bits(),
            seq: request_header.seq,
            pid: request_header.pid,
        };
        let family = if self.dst.is_ipv4() {
            CSocketAddrFamily::AF_INET
        } else {
            CSocketAddrFamily::AF_INET6
        };
        let body = CRtMsg {
            family: family as u8,
            dst_len: self.dst_len,
            src_len: 0,
            tos: 0,
            table: self.table as u8,
            protocol: self.protocol as u8,
            scope: self.scope as u8,
            type_: self.type_ as u8,
            flags: self.flags.bits(),
        };

        // Keep the common attributes in Linux's reporting order.
        // TODO: Report cache information, metrics, and IPv6 preferences.
        let mut attrs = vec![RouteAttr::Table(self.table)];
        if self.dst_len != 0 {
            attrs.push(RouteAttr::Dst(self.dst));
        }
        if self.metric != 0 || self.dst.is_ipv6() {
            attrs.push(RouteAttr::Priority(self.metric));
        }
        if let Some(source) = self.source {
            attrs.push(RouteAttr::PrefSrc(source));
        }
        if let Some(gateway) = self.gateway {
            attrs.push(RouteAttr::Gateway(gateway));
        }
        attrs.push(RouteAttr::Oif(self.output_index));
        RouteSegment::new(header, body, attrs)
    }
}

fn route_protocol<A: RouteAddressFamily>(entry: &RouteEntry<A>) -> RouteProtocol {
    // Routes imported from the interface are configured at boot; the others
    // are generated from the interface's own address and connected network.
    if entry.gateway().is_some() {
        RouteProtocol::Boot
    } else {
        RouteProtocol::Kernel
    }
}
