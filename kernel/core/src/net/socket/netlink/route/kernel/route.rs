// SPDX-License-Identifier: MPL-2.0

//! Handle route-related requests.

use core::{
    net::{IpAddr, Ipv4Addr},
    num::NonZeroU32,
};

use aster_bigtcp::{iface::InterfaceType, wire::IpCidr};

use super::util::{self, ResponseMode};
use crate::{
    net::{
        iface::{self, Iface},
        route::{self, RouteAddressFamily, RouteEntry, RouteTableId, RouteType},
        socket::netlink::{
            message::{CMsgSegHdr, CSegmentType, GetRequestFlags, SegHdrCommonFlags},
            route::message::{
                CRtMsg, RouteAttr, RouteAttrClass, RouteMessageFlags, RouteProtocol, RouteSegment,
                RtScope, RtnlSegment,
            },
        },
    },
    prelude::*,
    util::net::{CSocketAddrFamily, Protocol},
};

pub(super) fn do_get_route(
    request_segment: &RouteSegment,
    strict_check: bool,
) -> Result<Vec<RtnlSegment>> {
    validate_getroute_request(request_segment, strict_check)?;

    let dump_all = {
        let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
        flags.intersects(GetRequestFlags::DUMP)
    };
    let (mut response_segments, response_mode) = if dump_all {
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
        let response_segments = ipv4_segments
            .chain(ipv6_segments)
            .map(RtnlSegment::NewRoute)
            .collect();

        let response_mode = if filter_by.is_filtered {
            ResponseMode::FilteredDump
        } else {
            ResponseMode::Dump
        };
        (response_segments, response_mode)
    } else {
        let segment = lookup_to_new_route(request_segment)?;
        (vec![RtnlSegment::NewRoute(segment)], ResponseMode::Single)
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
    let body = request_segment.body();
    let flags = GetRequestFlags::from_bits_truncate(request_segment.header().flags);
    let dump_all = flags.intersects(GetRequestFlags::DUMP);
    if !dump_all {
        if (request_segment.header().len as usize) < size_of::<CMsgSegHdr>() + size_of::<CRtMsg>() {
            return_errno_with_message!(Errno::EINVAL, "the route message body is too short");
        }
        if body.family != CSocketAddrFamily::AF_INET as u8
            && body.family != CSocketAddrFamily::AF_INET6 as u8
        {
            return_errno_with_message!(
                Errno::EOPNOTSUPP,
                "the route address family is not supported"
            );
        }
    }
    if !strict_check {
        return Ok(());
    }

    // Linux accepts different headers and attributes for dumps and single-route queries.
    // References:
    // - <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/fib_frontend.c#L940>
    // - <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/route.c#L3221>
    // - <https://elixir.bootlin.com/linux/v7.1/source/net/ipv6/route.c#L6133>
    let allowed = if dump_all {
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
        [RouteAttrClass::TABLE, RouteAttrClass::OIF].as_slice()
    } else {
        let prefix_len = if body.family == CSocketAddrFamily::AF_INET as u8 {
            32
        } else {
            128
        };
        if (body.dst_len != 0 && body.dst_len != prefix_len)
            || (body.src_len != 0 && body.src_len != prefix_len)
            || body.table != 0
            || body.protocol != 0
            || body.scope != 0
            || body.type_ != 0
        {
            return_errno_with_message!(Errno::EINVAL, "the route lookup header is invalid");
        }
        let allowed_flags = if body.family == CSocketAddrFamily::AF_INET as u8 {
            RouteMessageFlags::NOTIFY
                | RouteMessageFlags::LOOKUP_TABLE
                | RouteMessageFlags::FIB_MATCH
        } else {
            RouteMessageFlags::FIB_MATCH
        };
        if body.flags & !allowed_flags.bits() != 0 {
            return_errno_with_message!(Errno::EINVAL, "the route lookup flags are invalid");
        }
        for attr in request_segment.attrs() {
            if (matches!(attr, RouteAttr::Dst(_)) && body.dst_len == 0)
                || (matches!(attr, RouteAttr::Src(_)) && body.src_len == 0)
            {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the route address requires a full prefix length"
                );
            }
        }
        [
            RouteAttrClass::DST,
            RouteAttrClass::SRC,
            RouteAttrClass::IIF,
            RouteAttrClass::OIF,
            RouteAttrClass::MARK,
            RouteAttrClass::UID,
            RouteAttrClass::IP_PROTO,
            RouteAttrClass::SPORT,
            RouteAttrClass::DPORT,
            RouteAttrClass::FLOWLABEL,
        ]
        .as_slice()
    };
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

fn lookup_to_new_route(request_segment: &RouteSegment) -> Result<RouteSegment> {
    let body = request_segment.body();
    let mut dst: IpAddr = if body.family == CSocketAddrFamily::AF_INET as u8 {
        Ipv4Addr::UNSPECIFIED.into()
    } else {
        core::net::Ipv6Addr::UNSPECIFIED.into()
    };
    let mut output_index = None;
    for attr in request_segment.attrs() {
        match attr {
            RouteAttr::Dst(address) => {
                if address.is_ipv4() != dst.is_ipv4() {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "the route destination family is invalid"
                    );
                }
                dst = *address;
            }
            RouteAttr::Oif(index) => output_index = NonZeroU32::new(*index),
            RouteAttr::Src(_) | RouteAttr::Iif(_) => {
                // TODO: Support source selection and input route lookups.
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "source and input interface selectors are not supported"
                );
            }
            RouteAttr::FlowLabel(label) => {
                const IPV6_FLOWLABEL_MASK: u32 = 0x000fffff;
                if body.family != CSocketAddrFamily::AF_INET6 as u8
                    || u32::from_be(*label) & !IPV6_FLOWLABEL_MASK != 0
                {
                    return_errno_with_message!(Errno::EINVAL, "the route flow label is invalid");
                }
            }
            RouteAttr::IpProtocol(protocol) => {
                // Linux only supports transport protocols whose flow keys it can construct.
                const IPPROTO_ICMPV6: u8 = 58;
                let icmp = if dst.is_ipv4() {
                    Protocol::IPPROTO_ICMP as u8
                } else {
                    IPPROTO_ICMPV6
                };
                if *protocol != Protocol::IPPROTO_TCP as u8
                    && *protocol != Protocol::IPPROTO_UDP as u8
                    && *protocol != icmp
                {
                    return_errno_with_message!(
                        Errno::EOPNOTSUPP,
                        "the route IP protocol is not supported"
                    );
                }
            }
            // The current routing rules do not select by mark, UID, or transport port.
            _ => {}
        }
    }
    if let Some(index) = output_index
        && !iface::iter_all_ifaces().any(|iface| iface.index() == index.get())
    {
        return_errno_with_message!(Errno::ENODEV, "the interface does not exist");
    }

    let flags = RouteMessageFlags::from_bits_truncate(body.flags);
    let info = match dst {
        IpAddr::V4(dst) => resolve_ipv4_route(dst, output_index, flags)?,
        IpAddr::V6(dst) => {
            let entry = route::lookup_ipv6_route(dst, output_index)?;
            let mut info = RouteInfo::from_entry(&entry);
            if !flags.contains(RouteMessageFlags::FIB_MATCH) {
                info.dst = dst.into();
                info.dst_len = 128;
                info.source = entry
                    .output_iface()
                    .ipv6_cidr()
                    .map(|cidr| cidr.address().into());
            }
            info
        }
    };
    Ok(info.into_segment(request_segment.header()))
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

fn resolve_ipv4_route(
    dst: Ipv4Addr,
    output_index: Option<NonZeroU32>,
    request_flags: RouteMessageFlags,
) -> Result<RouteInfo> {
    let output_iface = output_index
        .and_then(|index| iface::iter_all_ifaces().find(|iface| iface.index() == index.get()));
    let local_multicast = dst.octets()[..3] == [224, 0, 0];
    // These output routes do not require a FIB match on Linux.
    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/net/ipv4/route.c#L2786>.
    let mut info = if dst.is_unspecified() {
        let loopback = iface::iter_all_ifaces()
            .find(|iface| iface.type_() == InterfaceType::LOOPBACK)
            .ok_or_else(|| {
                Error::with_message(Errno::ENETUNREACH, "the loopback interface does not exist")
            })?;
        let mut info = ipv4_iface_route(dst, loopback.as_ref(), RouteType::Local);
        if let Some(source) = output_iface.and_then(|iface| iface.ipv4_cidr()) {
            info.source = Some(source.address().into());
        }
        info
    } else if let Some(iface) = output_iface.filter(|_| dst.is_broadcast() || local_multicast) {
        ipv4_iface_route(
            dst,
            iface.as_ref(),
            if dst.is_broadcast() {
                RouteType::Broadcast
            } else {
                RouteType::Multicast
            },
        )
    } else {
        let entry = route::lookup_ipv4_route(dst, output_index)?;
        RouteInfo::from_entry(&entry)
    };

    let has_fib_match = info.table != 0;
    if request_flags.contains(RouteMessageFlags::FIB_MATCH) && !has_fib_match {
        return_errno_with_message!(Errno::EHOSTUNREACH, "the output route has no FIB entry");
    }
    if !request_flags.contains(RouteMessageFlags::LOOKUP_TABLE) {
        info.table = RouteTableId::MAIN.get();
    }
    if dst.is_broadcast() {
        info.type_ = RouteType::Broadcast;
    } else if dst.is_multicast() {
        info.type_ = RouteType::Multicast;
    }
    if request_flags.contains(RouteMessageFlags::FIB_MATCH) {
        return Ok(info);
    }

    if dst.is_broadcast() || (dst.is_multicast() && info.dst_len < 4) {
        info.gateway = None;
    }
    info.dst = dst.into();
    info.dst_len = 32;
    info.protocol = RouteProtocol::Unspecified;
    info.scope = RtScope::UNIVERSE;
    info.flags = RouteMessageFlags::CLONED;
    if request_flags.contains(RouteMessageFlags::NOTIFY) {
        info.flags |= RouteMessageFlags::NOTIFY | RouteMessageFlags::IPV4_NOTIFY;
    }
    match info.type_ {
        RouteType::Local => {
            info.flags |= RouteMessageFlags::IPV4_LOCAL;
            // Local output is delivered through loopback even if the FIB entry
            // belongs to another interface.
            if let Some(iface) =
                iface::iter_all_ifaces().find(|iface| iface.type_() == InterfaceType::LOOPBACK)
            {
                info.output_index = iface.index();
            }
        }
        RouteType::Broadcast => {
            info.flags |= RouteMessageFlags::IPV4_LOCAL | RouteMessageFlags::IPV4_BROADCAST;
        }
        RouteType::Multicast => {
            info.flags |= RouteMessageFlags::IPV4_MULTICAST;
            // All IPv4 interfaces belong to the all-hosts group. Other group
            // memberships are not supported yet.
            if dst == Ipv4Addr::new(224, 0, 0, 1) {
                info.flags |= RouteMessageFlags::IPV4_LOCAL;
            }
        }
        RouteType::Unicast => {}
    }
    Ok(info)
}

fn ipv4_iface_route(dst: Ipv4Addr, iface: &Iface, type_: RouteType) -> RouteInfo {
    let source = if type_ == RouteType::Local {
        iface.ipv4_cidr().map(|cidr| cidr.address())
    } else {
        // Broadcasts and local multicasts select a source at link scope. A
        // loopback output interface therefore needs an address from another
        // interface, as in Linux's inet_select_addr().
        iface
            .ipv4_cidr()
            .map(|cidr| cidr.address())
            .filter(|address| !address.is_loopback())
            .or_else(|| {
                iface::iter_all_ifaces()
                    .filter_map(|iface| iface.ipv4_cidr())
                    .map(|cidr| cidr.address())
                    .find(|address| !address.is_loopback())
            })
    };
    RouteInfo {
        dst: dst.into(),
        dst_len: 32,
        source: source.map(Into::into),
        gateway: None,
        output_index: iface.index(),
        table: 0,
        metric: 0,
        protocol: RouteProtocol::Unspecified,
        scope: RtScope::UNIVERSE,
        type_,
        flags: RouteMessageFlags::empty(),
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
