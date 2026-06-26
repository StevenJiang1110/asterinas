// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{Ipv4Address, Ipv4Cidr};

use super::{
    cidrs_overlap,
    entry::{RouteDeleteKey, RouteEntry, RouteProtocol, RouteReplaceKey, RouteTableId},
    push_unique_cidr,
    rule::{RuleAction, RuleList},
    table::RouteTable,
};
use crate::prelude::*;

/// Maintains routing policy rules and route tables.
///
/// `RouteManager` owns the kernel's in-memory routing state. It currently
/// manages the IPv4 FIB, including bootstrap routes created from the initial
/// interface configuration and routes later changed through rtnetlink.
#[derive(Clone, Debug)]
pub(super) struct RouteManager {
    rules: RuleList,
    tables: BTreeMap<RouteTableId, RouteTable>,
}

/// Controls route insertion conflict handling.
#[derive(Clone, Copy, Debug)]
pub(in crate::net) struct RouteInsertOptions {
    create: bool,
    replace: bool,
    exclusive: bool,
}

impl RouteInsertOptions {
    pub(in crate::net) const fn new(create: bool, replace: bool, exclusive: bool) -> Self {
        Self {
            create,
            replace,
            exclusive,
        }
    }

    pub(super) const fn create_or_replace() -> Self {
        Self::new(true, true, false)
    }

    pub(super) const fn create(&self) -> bool {
        self.create
    }

    pub(super) const fn replace(&self) -> bool {
        self.replace
    }

    pub(super) const fn exclusive(&self) -> bool {
        self.exclusive
    }
}

/// A route lookup key.
///
/// The key currently supports destination and optional output-interface
/// selection. The constructor rejects other Linux selectors because policy
/// routing support is intentionally limited.
#[derive(Clone, Copy, Debug)]
pub struct RouteLookupKey {
    dst: Ipv4Address,
    oif_index: Option<u32>,
}

impl RouteLookupKey {
    /// Creates a lookup key for `dst`.
    pub fn new_dst(dst: Ipv4Address) -> Self {
        Self {
            dst,
            oif_index: None,
        }
    }

    /// Creates a lookup key from all parsed Linux lookup selectors.
    pub(in crate::net) fn new(
        dst: Ipv4Address,
        oif_index: Option<u32>,
        src: Option<Ipv4Address>,
        iif_index: Option<u32>,
        mark: u32,
        protocol: Option<u8>,
    ) -> Result<Self> {
        // Asterinas parses unsupported policy-routing selectors so requests
        // fail explicitly instead of being silently ignored.
        if src.is_some() || iif_index.is_some() || mark != 0 || protocol.is_some() {
            return_errno_with_message!(Errno::EOPNOTSUPP, "the route lookup key is not supported");
        }

        Ok(Self { dst, oif_index })
    }

    pub(super) fn dst(&self) -> Ipv4Address {
        self.dst
    }

    pub(in crate::net) fn oif_index(&self) -> Option<u32> {
        self.oif_index
    }
}

impl RouteManager {
    pub(super) fn new(bootstrap_routes: Vec<RouteEntry>) -> Self {
        let mut tables = BTreeMap::new();
        for table_id in [
            RouteTableId::LOCAL,
            RouteTableId::MAIN,
            RouteTableId::DEFAULT,
        ] {
            tables.insert(table_id, RouteTable::new());
        }

        let mut manager = Self {
            rules: RuleList::default(),
            tables,
        };
        for route in bootstrap_routes {
            let replace_key = route.replace_key();
            if let Err(err) =
                manager.insert(route, &replace_key, RouteInsertOptions::create_or_replace())
            {
                warn!("failed to install bootstrap IPv4 route: {:?}", err);
            }
        }
        manager
    }

    pub(super) fn insert_from_user(
        &mut self,
        route: RouteEntry,
        options: RouteInsertOptions,
    ) -> Result<Option<RouteEntry>> {
        let replace_key = route.replace_key();
        self.check_insert(&route, &replace_key, options)?;
        if options.replace() {
            self.check_replace_protected_route(&replace_key)?;
        }

        self.insert(route, &replace_key, options)
    }

    pub(super) fn route_to_replace_from_user(&self, key: &RouteReplaceKey) -> Option<RouteEntry> {
        self.tables
            .get(&key.table())
            .and_then(|table| table.route_to_replace(key))
            .cloned()
    }

    fn insert(
        &mut self,
        route: RouteEntry,
        replace_key: &RouteReplaceKey,
        options: RouteInsertOptions,
    ) -> Result<Option<RouteEntry>> {
        let table = self
            .tables
            .entry(route.table())
            .or_insert_with(RouteTable::new);
        table.insert(route, replace_key, options)
    }

    fn check_insert(
        &self,
        route: &RouteEntry,
        replace_key: &RouteReplaceKey,
        options: RouteInsertOptions,
    ) -> Result<()> {
        let Some(table) = self.tables.get(&route.table()) else {
            if !options.create() {
                return_errno_with_message!(Errno::ENOENT, "the route table does not exist");
            }
            return Ok(());
        };

        table.check_insert(replace_key, options)
    }

    fn check_replace_protected_route(&self, key: &RouteReplaceKey) -> Result<()> {
        let Some(table) = self.tables.get(&key.table()) else {
            return Ok(());
        };
        if table
            .route_to_replace(key)
            .is_some_and(|route| route.protocol() == RouteProtocol::KERNEL)
        {
            return_errno_with_message!(Errno::EOPNOTSUPP, "kernel routes cannot be replaced");
        }

        Ok(())
    }

    pub(super) fn delete_from_user(&mut self, key: &RouteDeleteKey) -> Result<RouteEntry> {
        let route = self.route_to_delete_from_user(key)?;
        if route.protocol() == RouteProtocol::KERNEL {
            return_errno_with_message!(Errno::EOPNOTSUPP, "kernel routes cannot be deleted");
        }

        self.tables
            .get_mut(&key.table())
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the route table does not exist"))?
            .delete(key)
    }

    pub(super) fn route_to_delete_from_user(&self, key: &RouteDeleteKey) -> Result<RouteEntry> {
        self.tables
            .get(&key.table())
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the route table does not exist"))?
            .route_to_delete(key)
            .cloned()
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the route does not exist"))
    }

    pub(super) fn dump(&self, table_filter: Option<RouteTableId>) -> Vec<RouteEntry> {
        match table_filter {
            Some(table_id) => self
                .tables
                .get(&table_id)
                .map(|table| table.entries().to_vec())
                .unwrap_or_default(),
            None => self
                .tables
                .values()
                .flat_map(|table| table.entries().iter().cloned())
                .collect(),
        }
    }

    pub(super) fn lookup_entry(&self, key: &RouteLookupKey) -> Result<RouteEntry> {
        for rule in self.rules.iter() {
            match rule.action() {
                RuleAction::Lookup => {
                    let Some(table_id) = rule.table() else {
                        continue;
                    };
                    let Some(table) = self.tables.get(&table_id) else {
                        continue;
                    };
                    if let Some(route) = table.lookup_with_key(key) {
                        return Ok(route);
                    }
                }
                RuleAction::Unreachable | RuleAction::Prohibit | RuleAction::Blackhole => {
                    return_errno_with_message!(Errno::ENETUNREACH, "the route rule rejects lookup");
                }
            }
        }

        return_errno_with_message!(Errno::ENETUNREACH, "no route to the destination")
    }

    pub(super) fn lookup_executable_gateway_entry_by_dst(
        &self,
        dst: Ipv4Cidr,
    ) -> Option<RouteEntry> {
        if self
            .tables
            .get(&RouteTableId::MAIN)
            .is_some_and(|table| table.has_lookup_route_covering(dst))
        {
            return self
                .tables
                .get(&RouteTableId::MAIN)
                .and_then(|table| table.lookup_best_exact_gateway(dst));
        }

        self.tables
            .get(&RouteTableId::DEFAULT)
            .and_then(|table| table.lookup_best_exact_gateway(dst))
    }

    pub(super) fn smoltcp_route_dsts_affected_by(&self, route: &RouteEntry) -> Vec<Ipv4Cidr> {
        let mut dsts = Vec::new();
        push_unique_cidr(&mut dsts, route.dst());

        for table_id in [RouteTableId::MAIN, RouteTableId::DEFAULT] {
            let Some(table) = self.tables.get(&table_id) else {
                continue;
            };
            for peer in table.entries() {
                if peer.has_raw_route() && cidrs_overlap(route.dst(), peer.dst()) {
                    push_unique_cidr(&mut dsts, peer.dst());
                }
            }
        }

        dsts
    }
}
