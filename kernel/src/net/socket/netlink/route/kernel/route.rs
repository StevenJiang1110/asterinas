// SPDX-License-Identifier: MPL-2.0

//! Handles route-related requests.

use aster_bigtcp::wire::{Ipv4Address, Ipv4Cidr};

use super::util;
use crate::{
    net::{
        route::{
            self, RouteDeleteKey, RouteEntry, RouteFlags, RouteInsertOptions, RouteLookupKey,
            RouteProtocol, RouteScope, RouteTableId, RouteType,
        },
        socket::netlink::{
            message::{
                CMsgSegHdr, CSegmentType, DeleteRequestFlags, ErrorSegment, GetRequestFlags,
                SegHdrCommonFlags,
            },
            route::message::{RouteAttr, RouteSegment, RouteSegmentBody, RtnlSegment},
        },
    },
    prelude::*,
    util::net::CSocketAddrFamily,
};

bitflags! {
    /// Modifiers for `RTM_NEWROUTE`.
    struct AddRouteFlags: u16 {
        /// Replaces an existing route.
        const REPLACE = 0x100;
        /// Fails when the route already exists.
        const EXCL = 0x200;
        /// Creates the route when it does not exist.
        const CREATE = 0x400;
    }
}

pub(super) fn do_get_route(request_segment: &RouteSegment) -> Result<Vec<RtnlSegment>> {
    let dump_all = GetRequestFlags::from_bits_truncate(request_segment.header().flags)
        .contains(GetRequestFlags::DUMP);

    let mut response_segments = if dump_all {
        ensure_ipv4_dump_request(request_segment)?;
        let filter = DumpFilter::new(request_segment);
        route::dump(filter.table)
            .into_iter()
            .filter(|entry| filter.matches(entry))
            .map(|entry| route_to_new_route(request_segment.header(), &entry))
            .map(RtnlSegment::NewRoute)
            .collect()
    } else {
        ensure_full_route_body(request_segment)?;
        ensure_ipv4_lookup_request(request_segment)?;
        let dst = route_lookup_dst(request_segment);
        let lookup_key = RouteLookupKey::new(dst, oif_index(request_segment), None, None, 0, None)?;
        if let Some(oif_index) = lookup_key.oif_index() {
            route::iface_by_index(oif_index).ok_or_else(|| {
                Error::with_message(Errno::ENODEV, "the route output iface does not exist")
            })?;
        }
        let entry = route::lookup(lookup_key)?;
        let route = if request_segment.body().flags.contains(RouteFlags::FIB_MATCH) {
            route_to_new_route(request_segment.header(), &entry)
        } else {
            let source = lookup_route_source(&entry)?;
            route_to_lookup_route(
                request_segment.header(),
                LookupTableReporting::from_flags(request_segment.body().flags),
                &entry,
                source,
                dst,
            )
        };
        vec![RtnlSegment::NewRoute(route)]
    };

    util::finish_response(request_segment.header(), dump_all, &mut response_segments);
    Ok(response_segments)
}

pub(super) fn do_new_route(request_segment: &RouteSegment) -> Result<Vec<RtnlSegment>> {
    ensure_full_route_body(request_segment)?;
    let entry = segment_to_route_entry(request_segment)?;
    route::insert_user_route(entry, insert_options(request_segment.header().flags)?)?;
    Ok(ack_if_requested(request_segment.header()))
}

pub(super) fn do_del_route(request_segment: &RouteSegment) -> Result<Vec<RtnlSegment>> {
    ensure_full_route_body(request_segment)?;
    DeleteRequestFlags::from_bits_truncate(request_segment.header().flags).check_unsupported()?;
    let key = segment_to_route_delete_key(request_segment)?;
    route::delete(&key)?;
    Ok(ack_if_requested(request_segment.header()))
}

fn route_to_new_route(request_header: &CMsgSegHdr, entry: &RouteEntry) -> RouteSegment {
    route_to_new_route_with_flags(request_header, entry, RouteFlags::empty())
}

fn route_to_new_route_with_flags(
    request_header: &CMsgSegHdr,
    entry: &RouteEntry,
    response_flags: RouteFlags,
) -> RouteSegment {
    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWROUTE as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };
    let body = RouteSegmentBody {
        family: CSocketAddrFamily::AF_INET as _,
        dst_len: entry.dst().prefix_len(),
        src_len: entry.src_len(),
        tos: entry.tos(),
        table: Some(entry.table()),
        protocol: entry.protocol(),
        scope: entry.scope(),
        type_: entry.type_(),
        flags: entry.flags() | response_flags,
    };
    let mut attrs = Vec::new();
    if entry.dst().prefix_len() != 0 {
        attrs.push(RouteAttr::Dst(entry.dst().address().octets()));
    }
    if let Some(gateway) = entry.gateway() {
        attrs.push(RouteAttr::Gateway(gateway.octets()));
    }
    if entry.oif_index() != 0 {
        attrs.push(RouteAttr::Oif(entry.oif_index()));
    }
    if entry.priority() != 0 {
        attrs.push(RouteAttr::Priority(entry.priority()));
    }
    attrs.push(RouteAttr::Table(entry.table().get()));

    RouteSegment::new(header, body, attrs)
}

fn route_to_lookup_route(
    request_header: &CMsgSegHdr,
    table_reporting: LookupTableReporting,
    entry: &RouteEntry,
    source: Ipv4Address,
    dst: Ipv4Address,
) -> RouteSegment {
    let table = table_reporting.response_table(entry);
    let header = CMsgSegHdr {
        len: 0,
        type_: CSegmentType::NEWROUTE as _,
        flags: SegHdrCommonFlags::empty().bits(),
        seq: request_header.seq,
        pid: request_header.pid,
    };
    let body = RouteSegmentBody {
        family: CSocketAddrFamily::AF_INET as _,
        dst_len: 32,
        src_len: 0,
        tos: 0,
        table: Some(table),
        protocol: RouteProtocol::UNSPEC,
        scope: entry.scope(),
        type_: entry.type_(),
        flags: RouteFlags::CLONED,
    };
    let mut attrs = vec![RouteAttr::Dst(dst.octets())];
    if let Some(gateway) = entry.gateway() {
        attrs.push(RouteAttr::Gateway(gateway.octets()));
    }
    if entry.oif_index() != 0 {
        attrs.push(RouteAttr::Oif(entry.oif_index()));
    }
    attrs.push(RouteAttr::PrefSrc(source.octets()));
    if entry.priority() != 0 {
        attrs.push(RouteAttr::Priority(entry.priority()));
    }
    if table_reporting == LookupTableReporting::Actual {
        attrs.push(RouteAttr::Table(table.get()));
    }

    RouteSegment::new(header, body, attrs)
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum LookupTableReporting {
    /// Reports the table where the lookup route was found.
    Actual,
    /// Reports `RT_TABLE_MAIN` for compatibility with default lookup replies.
    AsMain,
}

impl LookupTableReporting {
    fn from_flags(flags: RouteFlags) -> Self {
        if flags.contains(RouteFlags::LOOKUP_TABLE) {
            Self::Actual
        } else {
            Self::AsMain
        }
    }

    fn response_table(self, entry: &RouteEntry) -> RouteTableId {
        match self {
            Self::Actual => entry.table(),
            Self::AsMain => RouteTableId::MAIN,
        }
    }
}

fn segment_to_route_entry(segment: &RouteSegment) -> Result<RouteEntry> {
    ensure_ipv4_route_change(segment)?;
    ensure_no_unsupported_route_change_attrs(segment)?;

    let body = segment.body();
    if body.src_len != 0 || body.tos != 0 || !body.flags.is_empty() {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "source-prefix, TOS, and route flags are not supported"
        );
    }
    if !matches!(body.type_, RouteType::UNSPEC)
        && !matches!(
            body.type_,
            RouteType::UNICAST | RouteType::LOCAL | RouteType::BROADCAST
        )
    {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route type is not supported");
    }
    if body.protocol == RouteProtocol::KERNEL {
        return_errno_with_message!(Errno::EOPNOTSUPP, "kernel routes cannot be created");
    }

    let table = selected_table(segment, RouteTableId::MAIN);
    let dst_addr = route_dst_or_default(segment)?;
    let gateway = gateway(segment);
    let oif_index = oif_index(segment).unwrap_or(0);
    let priority = priority(segment);

    RouteEntry::new_full(
        Ipv4Cidr::new(dst_addr, body.dst_len),
        body.src_len,
        body.tos,
        gateway,
        oif_index,
        table,
        priority,
        body.protocol,
        body.scope,
        body.type_,
        body.flags,
    )
}

fn segment_to_route_delete_key(segment: &RouteSegment) -> Result<RouteDeleteKey> {
    ensure_ipv4_route_change(segment)?;
    ensure_no_unsupported_route_change_attrs(segment)?;

    let body = segment.body();
    if body.src_len != 0 || body.tos != 0 || !body.flags.is_empty() {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "source-prefix, TOS, and route flags are not supported"
        );
    }
    if !matches!(body.type_, RouteType::UNSPEC)
        && !matches!(
            body.type_,
            RouteType::UNICAST | RouteType::LOCAL | RouteType::BROADCAST
        )
    {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route type is not supported");
    }

    let table = selected_table(segment, RouteTableId::MAIN);
    let dst_addr = route_dst_or_default(segment)?;
    let gateway = gateway(segment);
    let oif_index = oif_index(segment);
    let priority = priority_attr(segment);

    let dst = Ipv4Cidr::new(dst_addr, body.dst_len);
    let _ = RouteEntry::new_full(
        dst,
        body.src_len,
        body.tos,
        gateway,
        oif_index.unwrap_or(0),
        table,
        priority.unwrap_or(0),
        body.protocol,
        body.scope,
        body.type_,
        body.flags,
    )?;
    Ok(RouteDeleteKey::new(
        table,
        dst,
        body.tos,
        (body.protocol != RouteProtocol::UNSPEC).then_some(body.protocol),
        (body.scope != RouteScope::UNIVERSE).then_some(body.scope),
        (body.type_ != RouteType::UNSPEC).then_some(body.type_),
        oif_index,
        gateway,
        priority,
    ))
}

fn lookup_route_source(entry: &RouteEntry) -> Result<Ipv4Address> {
    let iface = route::iface_by_index(entry.oif_index()).ok_or_else(|| {
        Error::with_message(Errno::ENODEV, "the route output iface does not exist")
    })?;
    iface.ipv4_addr().ok_or_else(|| {
        Error::with_message(
            Errno::EADDRNOTAVAIL,
            "the route output iface has no IPv4 address",
        )
    })
}

fn ensure_ipv4_route_change(segment: &RouteSegment) -> Result<()> {
    if segment.body().family != CSocketAddrFamily::AF_INET as i32 {
        return_errno_with_message!(Errno::EAFNOSUPPORT, "only IPv4 routes are supported");
    }

    Ok(())
}

fn ensure_ipv4_route_lookup(segment: &RouteSegment) -> Result<()> {
    if !matches!(
        segment.body().family,
        family if family == CSocketAddrFamily::AF_UNSPEC as i32
            || family == CSocketAddrFamily::AF_INET as i32
    ) {
        return_errno_with_message!(Errno::EAFNOSUPPORT, "only IPv4 routes are supported");
    }

    Ok(())
}

fn ensure_no_unsupported_route_change_attrs(segment: &RouteSegment) -> Result<()> {
    if segment.attrs().iter().any(|attr| {
        matches!(
            attr,
            RouteAttr::Src(_) | RouteAttr::Iif(_) | RouteAttr::PrefSrc(_)
        )
    }) {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route attribute is not supported");
    }

    Ok(())
}

fn ensure_full_route_body(segment: &RouteSegment) -> Result<()> {
    let payload_len = (segment.header().len as usize)
        .checked_sub(RouteSegment::HEADER_LEN)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the route message length is invalid"))?;
    if payload_len < RouteSegment::BODY_LEN {
        return_errno_with_message!(Errno::EINVAL, "the route message body is too short");
    }

    Ok(())
}

fn ensure_ipv4_lookup_request(segment: &RouteSegment) -> Result<()> {
    ensure_ipv4_route_lookup(segment)?;

    if segment.body().dst_len != 0 && segment.body().dst_len != 32 {
        return_errno_with_message!(Errno::EINVAL, "the route destination prefix is invalid");
    }
    if segment.body().src_len != 0 || segment.body().tos != 0 {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "source-prefix and TOS lookups are not supported"
        );
    }
    if segment.body().table.is_some()
        || segment.body().scope != RouteScope::UNIVERSE
        || !matches!(segment.body().type_, RouteType::UNSPEC | RouteType::UNICAST)
    {
        return_errno_with_message!(Errno::EINVAL, "the route lookup selector is invalid");
    }
    if segment.body().protocol != RouteProtocol::UNSPEC {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route protocol is not supported");
    }
    let unsupported_flags = segment.body().flags - RouteFlags::LOOKUP_TABLE - RouteFlags::FIB_MATCH;
    if !unsupported_flags.is_empty() {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route flags are not supported");
    }
    if segment.attrs().iter().any(|attr| {
        matches!(
            attr,
            RouteAttr::Src(_)
                | RouteAttr::Iif(_)
                | RouteAttr::Gateway(_)
                | RouteAttr::PrefSrc(_)
                | RouteAttr::Priority(_)
                | RouteAttr::Table(_)
        )
    }) {
        return_errno_with_message!(Errno::EOPNOTSUPP, "the route attribute is not supported");
    }

    Ok(())
}

fn ensure_ipv4_dump_request(segment: &RouteSegment) -> Result<()> {
    let Some(payload_len) = (segment.header().len as usize).checked_sub(RouteSegment::HEADER_LEN)
    else {
        return Ok(());
    };
    if payload_len < RouteSegment::BODY_LEN {
        return Ok(());
    }
    let unsupported_flags = segment.body().flags - RouteFlags::CLONED;
    if segment.body().dst_len != 0
        || segment.body().src_len != 0
        || segment.body().tos != 0
        || segment.body().scope != RouteScope::UNIVERSE
        || !unsupported_flags.is_empty()
    {
        return_errno_with_message!(Errno::EINVAL, "the route dump selector is invalid");
    }
    if segment.attrs().iter().any(|attr| {
        matches!(
            attr,
            RouteAttr::Dst(_)
                | RouteAttr::Src(_)
                | RouteAttr::Iif(_)
                | RouteAttr::Gateway(_)
                | RouteAttr::PrefSrc(_)
                | RouteAttr::Priority(_)
        )
    }) {
        return_errno_with_message!(Errno::EINVAL, "the route dump attribute is invalid");
    }
    Ok(())
}

fn route_dst(segment: &RouteSegment) -> Result<Ipv4Address> {
    if segment.body().dst_len == 0 {
        return Ok(Ipv4Address::UNSPECIFIED);
    }
    segment
        .attrs()
        .iter()
        .rev()
        .find_map(|attr| match attr {
            RouteAttr::Dst(dst) => Some(Ipv4Address::from_octets(*dst)),
            _ => None,
        })
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the route destination is missing"))
}

fn route_lookup_dst(segment: &RouteSegment) -> Ipv4Address {
    segment
        .attrs()
        .iter()
        .rev()
        .find_map(|attr| match attr {
            RouteAttr::Dst(dst) => Some(Ipv4Address::from_octets(*dst)),
            _ => None,
        })
        .unwrap_or(Ipv4Address::UNSPECIFIED)
}

fn route_dst_or_default(segment: &RouteSegment) -> Result<Ipv4Address> {
    if segment.body().dst_len == 0 {
        return Ok(Ipv4Address::UNSPECIFIED);
    }
    route_dst(segment)
}

fn gateway(segment: &RouteSegment) -> Option<Ipv4Address> {
    segment.attrs().iter().rev().find_map(|attr| match attr {
        RouteAttr::Gateway(gateway) => Some(Ipv4Address::from_octets(*gateway)),
        _ => None,
    })
}

fn oif_index(segment: &RouteSegment) -> Option<u32> {
    for attr in segment.attrs().iter().rev() {
        match attr {
            RouteAttr::Oif(0) => return None,
            RouteAttr::Oif(index) => return Some(*index),
            _ => {}
        }
    }

    None
}

fn priority(segment: &RouteSegment) -> u32 {
    priority_attr(segment).unwrap_or(0)
}

fn priority_attr(segment: &RouteSegment) -> Option<u32> {
    segment.attrs().iter().rev().find_map(|attr| match attr {
        RouteAttr::Priority(priority) => Some(*priority),
        _ => None,
    })
}

fn attr_table(segment: &RouteSegment) -> Option<Option<RouteTableId>> {
    segment.attrs().iter().rev().find_map(|attr| match attr {
        RouteAttr::Table(0) => Some(None),
        RouteAttr::Table(table) => Some(Some(RouteTableId::new(*table))),
        _ => None,
    })
}

fn selected_table(segment: &RouteSegment, default: RouteTableId) -> RouteTableId {
    attr_table(segment)
        .unwrap_or(segment.body().table)
        .unwrap_or(default)
}

fn insert_options(flags: u16) -> Result<RouteInsertOptions> {
    const NLM_F_APPEND: u16 = 0x800;
    if flags & NLM_F_APPEND != 0 {
        return_errno_with_message!(Errno::EOPNOTSUPP, "append route is not supported");
    }

    let flags = AddRouteFlags::from_bits_truncate(flags);
    Ok(RouteInsertOptions::new(
        flags.contains(AddRouteFlags::CREATE),
        flags.contains(AddRouteFlags::REPLACE),
        flags.contains(AddRouteFlags::EXCL),
    ))
}

struct DumpFilter {
    table: Option<RouteTableId>,
    protocol: RouteProtocol,
    type_: RouteType,
    oif_index: Option<u32>,
    cloned: bool,
}

impl DumpFilter {
    fn new(segment: &RouteSegment) -> Self {
        Self {
            table: attr_table(segment).unwrap_or(segment.body().table),
            protocol: segment.body().protocol,
            type_: segment.body().type_,
            oif_index: oif_index(segment),
            cloned: segment.body().flags.contains(RouteFlags::CLONED),
        }
    }

    fn matches(&self, entry: &RouteEntry) -> bool {
        // Asterinas does not maintain a route cache today, so requests for
        // cloned-only dumps correctly return no routes.
        !self.cloned
            && self.table.is_none_or(|table| entry.table() == table)
            && (self.protocol == RouteProtocol::UNSPEC || entry.protocol() == self.protocol)
            && (self.type_ == RouteType::UNSPEC || entry.type_() == self.type_)
            && self
                .oif_index
                .is_none_or(|oif_index| entry.oif_index() == oif_index)
    }
}

fn ack_if_requested(request_header: &CMsgSegHdr) -> Vec<RtnlSegment> {
    let flags = SegHdrCommonFlags::from_bits_truncate(request_header.flags);
    if flags.contains(SegHdrCommonFlags::ACK) {
        vec![RtnlSegment::Error(ErrorSegment::new_from_request(
            request_header,
            None,
        ))]
    } else {
        Vec::new()
    }
}
