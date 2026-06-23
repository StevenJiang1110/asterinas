// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{Ipv4Address, Ipv4Cidr};
use aster_util::ranged_integer::{RangedU8, RangedU32};

use super::RouteLookupKey;
use crate::prelude::*;

/// A route entry stored in the kernel IPv4 forwarding information base.
///
/// The entry mirrors the fields carried by Linux rtnetlink route messages, but
/// only the subset that can be executed by the current network stack is accepted
/// for lookup. `dst`, `table`, `type_`, `oif_index`, and `priority` identify the
/// route slot used for conflict detection. `gateway` distinguishes otherwise
/// identical routes. `src_len`, `tos`, and `flags` are stored because they are
/// user-visible rtnetlink fields and participate in validation, deletion, and
/// dumps even though non-zero values are not executable today. `has_raw_route`
/// records whether this entry is mirrored into the underlying smoltcp route
/// table.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RouteEntry {
    /// Destination network selected by longest-prefix matching.
    dst: Ipv4Cidr,
    /// Source prefix length from `rtmsg`; only zero is executable today.
    src_len: u8,
    /// Type of service selector from `rtmsg`; only zero is executable today.
    tos: u8,
    /// Next-hop gateway, if the route is not directly connected.
    gateway: Option<Ipv4Address>,
    /// Output interface index. Zero means unspecified in netlink requests.
    oif_index: u32,
    /// Linux route table that owns this entry.
    table: RouteTableId,
    /// Route metric. Lower values have higher priority for equal prefixes.
    priority: u32,
    /// Origin of the route.
    protocol: RouteProtocol,
    /// Visibility scope of the route destination.
    scope: RouteScope,
    /// Kernel route type such as unicast, local, or broadcast.
    type_: RouteType,
    /// Route flags carried in `rtmsg`.
    flags: RouteFlags,
    /// Whether this route is mirrored into smoltcp's raw route table.
    has_raw_route: bool,
}

/// A Linux route table identifier.
///
/// Full Linux route table identifiers are `u32` values carried by the `RTA_TABLE`
/// attribute. The `rtmsg` header has only an 8-bit table field, so tables above
/// `u8::MAX` are represented there as `RT_TABLE_UNSPEC` and must use
/// `RTA_TABLE` for the full value.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L354-L364>.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct RouteTableId(RangedU32<0, { u32::MAX }>);

impl RouteTableId {
    /// Identifies an unspecified route table in compact `rtmsg` headers.
    pub const UNSPEC: Self = Self::new(0);
    /// Identifies Linux's default route table.
    pub const DEFAULT: Self = Self::new(253);
    /// Identifies Linux's main route table.
    pub const MAIN: Self = Self::new(254);
    /// Identifies Linux's local route table.
    pub const LOCAL: Self = Self::new(255);

    /// Creates a route table identifier from a raw Linux table ID.
    pub const fn new(id: u32) -> Self {
        Self(RangedU32::new(id))
    }

    /// Returns the raw Linux table ID.
    pub const fn get(self) -> u32 {
        self.0.get()
    }

    /// Returns the compact `rtmsg` table value.
    pub const fn rtmsg_table(self) -> u8 {
        if self.0.get() <= u8::MAX as u32 {
            self.0.get() as u8
        } else {
            Self::UNSPEC.0.get() as u8
        }
    }

    /// Converts a compact `rtmsg` table value into a table ID.
    pub const fn from_rtmsg_table(table: u8) -> Option<Self> {
        if table == Self::UNSPEC.0.get() as u8 {
            None
        } else {
            Some(Self::new(table as u32))
        }
    }
}

/// Route protocol.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L270-L306>.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteProtocol(RangedU8<0, { u8::MAX }>);

impl RouteProtocol {
    /// Identifies an unspecified route protocol.
    pub const UNSPEC: Self = Self::new(0);
    /// Identifies routes created by the kernel.
    pub const KERNEL: Self = Self::new(2);
    /// Identifies routes created during boot.
    pub const BOOT: Self = Self::new(3);

    /// Creates a route protocol from a raw Linux protocol value.
    pub const fn new(protocol: u8) -> Self {
        Self(RangedU8::new(protocol))
    }

    /// Returns the raw Linux protocol value.
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

/// Route scope.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L318-L325>.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteScope(RangedU8<0, { u8::MAX }>);

impl RouteScope {
    /// Identifies globally reachable route destinations.
    pub const UNIVERSE: Self = Self::new(0);
    /// Identifies link-local route destinations.
    pub const LINK: Self = Self::new(253);
    /// Identifies host-local route destinations.
    pub const HOST: Self = Self::new(254);

    /// Creates a route scope from a raw Linux scope value.
    pub const fn new(scope: u8) -> Self {
        Self(RangedU8::new(scope))
    }

    /// Returns the raw Linux scope value.
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

/// Route type.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L252-L266>.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RouteType(RangedU8<0, { u8::MAX }>);

impl RouteType {
    /// Identifies an unspecified route type.
    pub const UNSPEC: Self = Self::new(0);
    /// Identifies a unicast route.
    pub const UNICAST: Self = Self::new(1);
    /// Identifies a local address route.
    pub const LOCAL: Self = Self::new(2);
    /// Identifies a broadcast address route.
    pub const BROADCAST: Self = Self::new(3);

    /// Creates a route type from a raw Linux route type value.
    pub const fn new(type_: u8) -> Self {
        Self(RangedU8::new(type_))
    }

    /// Returns the raw Linux route type value.
    pub const fn get(self) -> u8 {
        self.0.get()
    }
}

bitflags! {
    /// Route flags in `rtmsg`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.18/source/include/uapi/linux/rtnetlink.h#L327-L346>.
    pub struct RouteFlags: u32 {
        /// Requests notifications for route changes.
        const NOTIFY = 0x100;
        /// Identifies cloned or cached lookup results.
        const CLONED = 0x200;
        /// Identifies equal-cost multipath balancing.
        const EQUALIZE = 0x400;
        /// Identifies a prefix route.
        const PREFIX = 0x800;
        /// Requests the lookup table to be reported.
        const LOOKUP_TABLE = 0x1000;
        /// Requests the matched FIB route instead of a cloned lookup result.
        const FIB_MATCH = 0x2000;
    }
}

/// A key parsed from an IPv4 `RTM_DELROUTE` request.
///
/// Linux route deletion does not always require every field that was used to
/// create the route. `table`, `dst`, and `tos` come from required `rtmsg`
/// fields after defaults are applied. `protocol`, `scope`, `type_`,
/// `oif_index`, `gateway`, and `priority` are optional selectors; absent values
/// do not restrict deletion.
#[derive(Debug)]
pub(in crate::net) struct RouteDeleteKey {
    table: RouteTableId,
    dst: Ipv4Cidr,
    tos: u8,
    protocol: Option<RouteProtocol>,
    scope: Option<RouteScope>,
    type_: Option<RouteType>,
    oif_index: Option<u32>,
    gateway: Option<Ipv4Address>,
    priority: Option<u32>,
}

impl RouteDeleteKey {
    #[expect(clippy::too_many_arguments)]
    pub(in crate::net) fn new(
        table: RouteTableId,
        dst: Ipv4Cidr,
        tos: u8,
        protocol: Option<RouteProtocol>,
        scope: Option<RouteScope>,
        type_: Option<RouteType>,
        oif_index: Option<u32>,
        gateway: Option<Ipv4Address>,
        priority: Option<u32>,
    ) -> Self {
        Self {
            table,
            dst,
            tos,
            protocol,
            scope,
            type_,
            oif_index,
            gateway,
            priority,
        }
    }

    pub(super) fn table(&self) -> RouteTableId {
        self.table
    }
}

/// A key used to find the route replaced by `RTM_NEWROUTE`.
#[derive(Clone, Debug)]
pub(super) struct RouteReplaceKey {
    table: RouteTableId,
    dst: Ipv4Cidr,
    tos: u8,
    gateway: Option<Ipv4Address>,
    oif_index: u32,
    priority: u32,
    type_: RouteType,
}

impl RouteReplaceKey {
    pub(super) fn table(&self) -> RouteTableId {
        self.table
    }
}

impl RouteEntry {
    /// Creates an executable IPv4 route entry with default selectors.
    pub fn new(
        dst: Ipv4Cidr,
        table: RouteTableId,
        protocol: RouteProtocol,
        scope: RouteScope,
        type_: RouteType,
        oif_index: u32,
        gateway: Option<Ipv4Address>,
    ) -> Result<Self> {
        Self::new_full(
            dst,
            0,
            0,
            gateway,
            oif_index,
            table,
            0,
            protocol,
            scope,
            type_,
            RouteFlags::empty(),
        )
    }

    /// Creates an IPv4 route entry from full rtnetlink route fields.
    #[expect(clippy::too_many_arguments)]
    pub fn new_full(
        dst: Ipv4Cidr,
        src_len: u8,
        tos: u8,
        gateway: Option<Ipv4Address>,
        oif_index: u32,
        table: RouteTableId,
        priority: u32,
        protocol: RouteProtocol,
        scope: RouteScope,
        type_: RouteType,
        flags: RouteFlags,
    ) -> Result<Self> {
        if dst.prefix_len() > 32 || src_len > 32 {
            return_errno_with_message!(Errno::EINVAL, "the IPv4 route prefix length is invalid");
        }
        if dst != dst.network() {
            return_errno_with_message!(
                Errno::EINVAL,
                "the IPv4 route destination is not canonical"
            );
        }
        let has_raw_route = type_ == RouteType::UNICAST
            && gateway.is_some()
            && matches!(table, RouteTableId::MAIN | RouteTableId::DEFAULT);
        Ok(Self {
            dst: dst.network(),
            src_len,
            tos,
            gateway,
            oif_index,
            table,
            priority,
            protocol,
            scope,
            type_,
            flags,
            has_raw_route,
        })
    }

    /// Returns the destination CIDR.
    pub fn dst(&self) -> Ipv4Cidr {
        self.dst
    }

    /// Returns the source prefix length selector.
    pub fn src_len(&self) -> u8 {
        self.src_len
    }

    /// Returns the type-of-service selector.
    pub fn tos(&self) -> u8 {
        self.tos
    }

    /// Returns the next-hop gateway.
    pub fn gateway(&self) -> Option<Ipv4Address> {
        self.gateway
    }

    /// Returns the output interface index.
    pub fn oif_index(&self) -> u32 {
        self.oif_index
    }

    /// Returns the route table ID.
    pub fn table(&self) -> RouteTableId {
        self.table
    }

    /// Returns the route metric.
    pub fn priority(&self) -> u32 {
        self.priority
    }

    /// Returns the route protocol.
    pub fn protocol(&self) -> RouteProtocol {
        self.protocol
    }

    /// Returns the route scope.
    pub fn scope(&self) -> RouteScope {
        self.scope
    }

    /// Returns the route type.
    pub fn type_(&self) -> RouteType {
        self.type_
    }

    /// Returns the route flags.
    pub fn flags(&self) -> RouteFlags {
        self.flags
    }

    /// Returns whether this route is mirrored into smoltcp's route table.
    pub(super) fn has_raw_route(&self) -> bool {
        self.has_raw_route
    }

    pub(super) fn replace_key(&self) -> RouteReplaceKey {
        RouteReplaceKey {
            table: self.table,
            dst: self.dst,
            tos: self.tos,
            gateway: self.gateway,
            oif_index: self.oif_index,
            priority: self.priority,
            type_: self.type_,
        }
    }

    pub(super) fn matches_identity_key(&self, key: &RouteReplaceKey) -> bool {
        self.matches_route_slot_key(key) && self.gateway == key.gateway
    }

    pub(super) fn matches_route_slot_key(&self, key: &RouteReplaceKey) -> bool {
        self.table == key.table
            && self.dst == key.dst
            && self.tos == key.tos
            && self.type_ == key.type_
            && self.oif_index == key.oif_index
            && self.priority == key.priority
    }

    pub(super) fn matches_replacement_key(&self, key: &RouteReplaceKey) -> bool {
        self.table == key.table
            && self.dst == key.dst
            && self.tos == key.tos
            && self.type_ == key.type_
            && self.priority == key.priority
    }

    pub(super) fn matches_delete_key(&self, key: &RouteDeleteKey) -> bool {
        self.table == key.table
            && self.dst == key.dst
            && self.tos == key.tos
            && key
                .protocol
                .is_none_or(|protocol| self.protocol == protocol)
            && key.scope.is_none_or(|scope| self.scope == scope)
            && key.type_.is_none_or(|type_| self.type_ == type_)
            && key
                .oif_index
                .is_none_or(|oif_index| self.oif_index == oif_index)
            && key
                .priority
                .is_none_or(|priority| self.priority == priority)
            && key
                .gateway
                .is_none_or(|gateway| self.gateway == Some(gateway))
    }

    pub(super) fn matches_lookup(&self, key: &RouteLookupKey) -> bool {
        self.matches_lookup_dst(key.dst())
            && key
                .oif_index()
                .is_none_or(|oif_index| self.oif_index == oif_index)
    }

    pub(super) fn matches_lookup_dst(&self, dst: Ipv4Address) -> bool {
        matches!(
            self.type_,
            RouteType::UNICAST | RouteType::LOCAL | RouteType::BROADCAST
        ) && self.src_len == 0
            && self.tos == 0
            && self.flags.is_empty()
            && self.dst.contains_addr(&dst)
    }
}
