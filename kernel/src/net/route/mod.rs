// SPDX-License-Identifier: MPL-2.0

//! Routing table support.
//!
//! The top-level route module owns the kernel routing table and exposes the
//! operations used by socket lookup and rtnetlink. The current implementation
//! stores IPv4 routes only, but callers go through this module so IPv6 can be
//! added under the same routing abstraction later.

use aster_bigtcp::{
    iface::{InterfaceFlags, InterfaceType},
    wire::{IpAddress, IpEndpoint, Ipv4Address, Ipv4Cidr},
};
use spin::Once;

use self::manager::RouteManager;
use super::iface::{self, Iface};
use crate::prelude::*;

mod entry;
mod manager;
mod rule;
mod table;

pub(in crate::net) use entry::RouteDeleteKey;
pub use entry::{RouteEntry, RouteFlags, RouteProtocol, RouteScope, RouteTableId, RouteType};
pub(in crate::net) use manager::RouteInsertOptions;
pub use manager::RouteLookupKey;

static ROUTE_MANAGER: Once<RwMutex<RouteManager>> = Once::new();
const LIMITED_BROADCAST_ADDR: Ipv4Address = Ipv4Address::new(255, 255, 255, 255);

/// Initializes routes from the currently configured interfaces.
pub fn init_from_ifaces() {
    let routes = iface::iter_all_ifaces()
        .filter_map(|iface| match bootstrap_routes_for_iface(iface) {
            Ok(routes) => Some(routes),
            Err(err) => {
                warn!(
                    "failed to collect bootstrap IPv4 routes for iface {}: {:?}",
                    iface.index(),
                    err
                );
                None
            }
        })
        .flatten()
        .collect();

    ROUTE_MANAGER.call_once(|| RwMutex::new(RouteManager::new(routes)));
}

fn bootstrap_routes_for_iface(iface: &Arc<Iface>) -> Result<Vec<RouteEntry>> {
    let Some(ipv4_addr) = iface.ipv4_addr() else {
        return Ok(Vec::new());
    };
    let Some(prefix_len) = iface.prefix_len() else {
        return Ok(Vec::new());
    };

    let iface_cidr = Ipv4Cidr::new(ipv4_addr, prefix_len);
    let mut routes = Vec::new();
    routes.push(RouteEntry::new(
        iface_cidr.network(),
        RouteTableId::MAIN,
        RouteProtocol::KERNEL,
        RouteScope::LINK,
        RouteType::UNICAST,
        iface.index(),
        None,
    )?);

    let local_dst = if iface.type_() == InterfaceType::LOOPBACK {
        iface_cidr.network()
    } else {
        Ipv4Cidr::new(ipv4_addr, 32)
    };
    routes.push(RouteEntry::new(
        local_dst,
        RouteTableId::LOCAL,
        RouteProtocol::KERNEL,
        RouteScope::HOST,
        RouteType::LOCAL,
        iface.index(),
        None,
    )?);

    routes.push(RouteEntry::new(
        Ipv4Cidr::new(LIMITED_BROADCAST_ADDR, 32),
        RouteTableId::LOCAL,
        RouteProtocol::KERNEL,
        RouteScope::LINK,
        RouteType::BROADCAST,
        iface.index(),
        None,
    )?);

    if let Some(broadcast_addr) = iface.broadcast_addr() {
        routes.push(RouteEntry::new(
            Ipv4Cidr::new(broadcast_addr, 32),
            RouteTableId::LOCAL,
            RouteProtocol::KERNEL,
            RouteScope::LINK,
            RouteType::BROADCAST,
            iface.index(),
            None,
        )?);
    }

    for (dst, gateway) in iface.ipv4_routes() {
        routes.push(RouteEntry::new(
            dst,
            RouteTableId::MAIN,
            RouteProtocol::BOOT,
            RouteScope::UNIVERSE,
            RouteType::UNICAST,
            iface.index(),
            Some(gateway),
        )?);
    }

    Ok(routes)
}

/// Adds or replaces an IPv4 route from a user request.
pub(in crate::net) fn insert_user_route(
    route: RouteEntry,
    options: RouteInsertOptions,
) -> Result<()> {
    validate_executable_route(&route)?;

    let mut route_manager = manager().write();
    let mut dsts = route_manager.smoltcp_route_dsts_affected_by(&route);
    push_unique_cidr(&mut dsts, route.dst());
    let replace_key = route.replace_key();
    if let Some(route_to_replace) = route_manager.route_to_replace_from_user(&replace_key) {
        extend_unique_cidrs(
            &mut dsts,
            route_manager.smoltcp_route_dsts_affected_by(&route_to_replace),
        );
    }
    let old_selected_routes = selected_smoltcp_routes(&route_manager, &dsts);

    let replaced = route_manager.insert_from_user(route.clone(), options)?;
    extend_unique_cidrs(
        &mut dsts,
        route_manager.smoltcp_route_dsts_affected_by(&route),
    );
    if let Some(replaced) = &replaced {
        extend_unique_cidrs(
            &mut dsts,
            route_manager.smoltcp_route_dsts_affected_by(replaced),
        );
    }

    refresh_smoltcp_routes(old_selected_routes, &route_manager, &dsts)
}

/// Deletes an IPv4 route.
pub(in crate::net) fn delete(key: &RouteDeleteKey) -> Result<RouteEntry> {
    let mut manager = manager().write();
    let route_to_delete = manager.route_to_delete_from_user(key)?;
    let mut dsts = manager.smoltcp_route_dsts_affected_by(&route_to_delete);
    let old_selected_routes = selected_smoltcp_routes(&manager, &dsts);

    let removed = manager.delete_from_user(key)?;
    extend_unique_cidrs(&mut dsts, manager.smoltcp_route_dsts_affected_by(&removed));
    refresh_smoltcp_routes(old_selected_routes, &manager, &dsts)?;
    Ok(removed)
}

/// Dumps IPv4 routes.
pub fn dump(table_filter: Option<RouteTableId>) -> Vec<RouteEntry> {
    manager().read().dump(table_filter)
}

/// Looks up an IPv4 route.
pub fn lookup(key: RouteLookupKey) -> Result<RouteEntry> {
    manager().read().lookup_entry(&key)
}

/// Determines if the endpoint is routed to an IPv4 broadcast address.
///
/// Broadcast addresses are represented as `RTN_BROADCAST` entries in the local
/// route table. Keeping this check on top of route lookup avoids a second,
/// stale copy of interface broadcast addresses.
pub fn is_broadcast_endpoint(endpoint: &IpEndpoint) -> bool {
    let IpAddress::Ipv4(ipv4_addr) = endpoint.addr else {
        return false;
    };

    manager()
        .read()
        .lookup_entry(&RouteLookupKey::new_dst(ipv4_addr))
        .is_ok_and(|route| route.type_() == RouteType::BROADCAST)
}

/// Returns an interface by index.
pub fn iface_by_index(index: u32) -> Option<Arc<Iface>> {
    iface::iter_all_ifaces()
        .find(|iface| iface.index() == index)
        .map(Clone::clone)
}

fn validate_executable_route(route: &RouteEntry) -> Result<()> {
    if route.src_len() != 0 || route.tos() != 0 || !route.flags().is_empty() {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "source-prefix, TOS, and route flags are not supported"
        );
    }

    match route.type_() {
        RouteType::UNICAST => {
            if route.oif_index() == 0 {
                return_errno_with_message!(Errno::EINVAL, "the output interface is required");
            }
            let iface = iface_by_index(route.oif_index()).ok_or_else(|| {
                Error::with_message(Errno::ENODEV, "the route output iface does not exist")
            })?;
            if let Some(gateway) = route.gateway() {
                validate_unicast_gateway(route, &iface, gateway)?;
            } else if iface.ipv4_addr().is_none() {
                return_errno_with_message!(
                    Errno::EADDRNOTAVAIL,
                    "the route output iface has no IPv4 address"
                );
            }
        }
        RouteType::LOCAL | RouteType::BROADCAST => {
            if route.gateway().is_some() {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "non-unicast routes cannot have a gateway"
                );
            }
            if route.oif_index() == 0 {
                return_errno_with_message!(Errno::EINVAL, "the output interface is required");
            }
            let iface = iface_by_index(route.oif_index()).ok_or_else(|| {
                Error::with_message(Errno::ENODEV, "the route output iface does not exist")
            })?;
            if !is_interface_host_route(route, &iface) {
                return_errno_with_message!(
                    Errno::EOPNOTSUPP,
                    "arbitrary local and broadcast routes are not supported"
                );
            }
        }
        RouteType::UNSPEC => {
            return_errno_with_message!(Errno::EINVAL, "the route type is invalid");
        }
        _ => {
            return_errno_with_message!(Errno::EOPNOTSUPP, "the route type is not supported");
        }
    }

    Ok(())
}

fn validate_unicast_gateway(
    route: &RouteEntry,
    iface: &Arc<Iface>,
    gateway: Ipv4Address,
) -> Result<()> {
    if gateway.is_broadcast() || gateway.is_multicast() || gateway.is_unspecified() {
        return_errno_with_message!(Errno::EINVAL, "the route gateway is invalid");
    }
    let Some(ipv4_addr) = iface.ipv4_addr() else {
        return_errno_with_message!(
            Errno::EADDRNOTAVAIL,
            "the route output iface has no IPv4 address"
        );
    };
    let Some(prefix_len) = iface.prefix_len() else {
        return_errno_with_message!(
            Errno::EADDRNOTAVAIL,
            "the route output iface has no IPv4 prefix length"
        );
    };

    let iface_cidr = Ipv4Cidr::new(ipv4_addr, prefix_len);
    if !iface_cidr.contains_addr(&gateway) {
        return_errno_with_message!(Errno::ENETUNREACH, "the route gateway is unreachable");
    }
    if !is_usable_gateway_addr(gateway, &iface_cidr, ipv4_addr) {
        return_errno_with_message!(Errno::EINVAL, "the route gateway is invalid");
    }
    if route.dst().prefix_len() >= iface_cidr.prefix_len() && cidrs_overlap(iface_cidr, route.dst())
    {
        return_errno_with_message!(
            Errno::EOPNOTSUPP,
            "same-link gateway routes are not supported"
        );
    }

    Ok(())
}

fn is_usable_gateway_addr(
    gateway: Ipv4Address,
    iface_cidr: &Ipv4Cidr,
    iface_addr: Ipv4Address,
) -> bool {
    if gateway == iface_addr {
        return false;
    }

    if iface_cidr.prefix_len() < 31 && gateway == iface_cidr.network().address() {
        return false;
    }

    iface_cidr.broadcast() != Some(gateway)
}

pub(super) fn cidrs_overlap(first: Ipv4Cidr, second: Ipv4Cidr) -> bool {
    first.contains_addr(&second.address()) || second.contains_addr(&first.address())
}

pub(super) fn push_unique_cidr(dsts: &mut Vec<Ipv4Cidr>, dst: Ipv4Cidr) {
    if !dsts.contains(&dst) {
        dsts.push(dst);
    }
}

fn extend_unique_cidrs(dsts: &mut Vec<Ipv4Cidr>, new_dsts: Vec<Ipv4Cidr>) {
    for dst in new_dsts {
        push_unique_cidr(dsts, dst);
    }
}

fn selected_smoltcp_routes(
    manager: &RouteManager,
    dsts: &[Ipv4Cidr],
) -> Vec<(Ipv4Cidr, Option<RouteEntry>)> {
    dsts.iter()
        .copied()
        .map(|dst| (dst, manager.lookup_executable_gateway_entry_by_dst(dst)))
        .collect()
}

fn refresh_smoltcp_routes(
    old_selected_routes: Vec<(Ipv4Cidr, Option<RouteEntry>)>,
    manager: &RouteManager,
    dsts: &[Ipv4Cidr],
) -> Result<()> {
    for dst in dsts {
        let old_selected = old_selected_routes
            .iter()
            .find_map(|(old_dst, old_route)| (*old_dst == *dst).then_some(old_route));
        refresh_smoltcp_route(
            *dst,
            old_selected.and_then(Option::as_ref),
            manager
                .lookup_executable_gateway_entry_by_dst(*dst)
                .as_ref(),
        )?;
    }
    Ok(())
}

fn refresh_smoltcp_route(
    dst: Ipv4Cidr,
    old_selected: Option<&RouteEntry>,
    selected: Option<&RouteEntry>,
) -> Result<()> {
    let old_iface_index = old_selected
        .filter(|route| route.has_raw_route())
        .map(|route| route.oif_index());
    let new_iface_index = selected
        .filter(|route| route.has_raw_route())
        .map(|route| route.oif_index());

    if let Some(iface_index) = old_iface_index
        && let Some(iface) = iface_by_index(iface_index)
    {
        iface.remove_ipv4_route(dst);
    }

    if new_iface_index != old_iface_index
        && let Some(iface_index) = new_iface_index
        && let Some(iface) = iface_by_index(iface_index)
    {
        iface.remove_ipv4_route(dst);
    }

    if let Some(selected) = selected.filter(|route| route.has_raw_route())
        && let Some(gateway) = selected.gateway()
    {
        let iface = iface_by_index(selected.oif_index()).ok_or_else(|| {
            Error::with_message(Errno::ENODEV, "the route output iface does not exist")
        })?;
        iface
            .add_ipv4_route(selected.dst(), gateway)
            .map_err(|_| Error::with_message(Errno::ENOMEM, "the smoltcp route table is full"))?;
    }

    Ok(())
}

fn is_interface_host_route(route: &RouteEntry, iface: &Arc<Iface>) -> bool {
    match route.type_() {
        RouteType::LOCAL => {
            if route.dst().prefix_len() == 32 {
                iface.ipv4_addr() == Some(route.dst().address())
            } else if iface.type_() == InterfaceType::LOOPBACK {
                iface
                    .ipv4_addr()
                    .zip(iface.prefix_len())
                    .map(|(addr, prefix_len)| Ipv4Cidr::new(addr, prefix_len).network())
                    == Some(route.dst())
            } else {
                false
            }
        }
        RouteType::BROADCAST => {
            if route.dst().prefix_len() != 32 {
                return false;
            }
            if !iface.flags().contains(InterfaceFlags::BROADCAST) {
                return false;
            }
            if route.dst().address() == LIMITED_BROADCAST_ADDR {
                iface.ipv4_addr().is_some()
            } else {
                iface.broadcast_addr() == Some(route.dst().address())
            }
        }
        _ => false,
    }
}

fn manager() -> &'static RwMutex<RouteManager> {
    ROUTE_MANAGER.call_once(|| RwMutex::new(RouteManager::new(Vec::new())))
}
