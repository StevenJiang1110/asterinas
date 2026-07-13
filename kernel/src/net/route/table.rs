// SPDX-License-Identifier: MPL-2.0

use super::{
    RouteInsertOptions, RouteLookupKey,
    entry::{RouteDeleteKey, RouteEntry, RouteReplaceKey},
};
use crate::prelude::*;

/// One Linux IPv4 route table.
///
/// Although the type is named as a table, it intentionally stores routes in a
/// `Vec` rather than a key-value map. Route lookup needs longest-prefix and
/// lowest-metric selection, route replacement has Linux-compatible matching
/// rules that are not a single unique key, and rtnetlink dumps must enumerate
/// entries. The current per-table route count is small, so a linear scan keeps
/// those semantics explicit without prematurely choosing an index structure.
#[derive(Clone, Debug)]
pub(super) struct RouteTable {
    entries: Vec<RouteEntry>,
}

impl RouteTable {
    pub(super) fn new() -> Self {
        Self {
            entries: Vec::new(),
        }
    }

    pub(super) fn entries(&self) -> &[RouteEntry] {
        &self.entries
    }

    pub(super) fn insert(
        &mut self,
        route: RouteEntry,
        replace_key: &RouteReplaceKey,
        options: RouteInsertOptions,
    ) -> Result<Option<RouteEntry>> {
        self.check_insert(replace_key, options)?;

        let existing_index = self
            .entries
            .iter()
            .position(|entry| entry.matches_identity_key(replace_key));
        if let Some(index) = existing_index {
            let replaced = core::mem::replace(&mut self.entries[index], route);
            return Ok(Some(replaced));
        }

        if options.replace()
            && let Some(index) = self.route_to_replace_index(replace_key)
        {
            let replaced = core::mem::replace(&mut self.entries[index], route);
            return Ok(Some(replaced));
        }

        self.entries.push(route);
        Ok(None)
    }

    pub(super) fn check_insert(
        &self,
        replace_key: &RouteReplaceKey,
        options: RouteInsertOptions,
    ) -> Result<()> {
        let route_slot_exists = self
            .entries
            .iter()
            .any(|entry| entry.matches_route_slot_key(replace_key));

        if route_slot_exists {
            if options.exclusive() {
                return_errno_with_message!(Errno::EEXIST, "the route already exists");
            }
            if !options.replace() {
                return_errno_with_message!(Errno::EEXIST, "the route already exists");
            }
            return Ok(());
        }

        if options.replace() && self.route_to_replace_index(replace_key).is_some() {
            if options.exclusive() {
                return_errno_with_message!(Errno::EEXIST, "the route already exists");
            }
            return Ok(());
        }

        if !options.create() {
            return_errno_with_message!(Errno::ENOENT, "the route does not exist");
        }

        Ok(())
    }

    pub(super) fn route_to_replace(&self, key: &RouteReplaceKey) -> Option<&RouteEntry> {
        self.entries
            .iter()
            .find(|entry| entry.matches_identity_key(key))
            .or_else(|| {
                self.entries
                    .iter()
                    .find(|entry| entry.matches_route_slot_key(key))
            })
            .or_else(|| self.single_replacement_slot_route(key))
    }

    fn route_to_replace_index(&self, key: &RouteReplaceKey) -> Option<usize> {
        self.entries
            .iter()
            .position(|entry| entry.matches_identity_key(key))
            .or_else(|| {
                self.entries
                    .iter()
                    .position(|entry| entry.matches_route_slot_key(key))
            })
            .or_else(|| self.single_replacement_slot_index(key))
    }

    fn single_replacement_slot_route(&self, key: &RouteReplaceKey) -> Option<&RouteEntry> {
        let mut matches = self
            .entries
            .iter()
            .filter(|entry| entry.matches_replacement_key(key));
        let route = matches.next()?;
        matches.next().is_none().then_some(route)
    }

    fn single_replacement_slot_index(&self, key: &RouteReplaceKey) -> Option<usize> {
        let mut matches = self
            .entries
            .iter()
            .enumerate()
            .filter(|(_, entry)| entry.matches_replacement_key(key));
        let (index, _) = matches.next()?;
        matches.next().is_none().then_some(index)
    }

    pub(super) fn route_to_delete(&self, key: &RouteDeleteKey) -> Option<&RouteEntry> {
        self.entries
            .iter()
            .find(|entry| entry.matches_delete_key(key))
    }

    pub(super) fn delete(&mut self, key: &RouteDeleteKey) -> Result<RouteEntry> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.matches_delete_key(key))
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the route does not exist"))?;
        Ok(self.entries.remove(index))
    }

    pub(super) fn lookup_with_key(&self, key: &RouteLookupKey) -> Option<RouteEntry> {
        let mut best = None;
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.matches_lookup(key))
        {
            if best.is_none_or(|best| is_better_lookup_match(entry, best)) {
                best = Some(entry);
            }
        }

        best.cloned()
    }

    pub(super) fn lookup_best_exact_gateway(
        &self,
        dst: aster_bigtcp::wire::Ipv4Cidr,
    ) -> Option<RouteEntry> {
        let mut best = None;
        for entry in self
            .entries
            .iter()
            .filter(|entry| entry.dst() == dst && entry.gateway().is_some())
        {
            if best.is_none_or(|best| is_better_lookup_match(entry, best)) {
                best = Some(entry);
            }
        }

        best.cloned()
    }

    /// Returns whether a lookup-capable route covers `dst`.
    ///
    /// This is used to preserve Linux routing rule order: a matching `MAIN`
    /// table route blocks fall-through to `DEFAULT`, even when that `MAIN`
    /// route has no gateway and therefore cannot be mirrored into smoltcp.
    pub(super) fn has_lookup_route_covering(&self, dst: aster_bigtcp::wire::Ipv4Cidr) -> bool {
        self.entries.iter().any(|entry| {
            entry.matches_lookup_dst(dst.address()) && entry.dst().prefix_len() <= dst.prefix_len()
        })
    }
}

fn is_better_lookup_match(candidate: &RouteEntry, best: &RouteEntry) -> bool {
    (
        candidate.dst().prefix_len(),
        core::cmp::Reverse(candidate.priority()),
    ) > (best.dst().prefix_len(), core::cmp::Reverse(best.priority()))
}
