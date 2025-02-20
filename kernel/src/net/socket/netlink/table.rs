// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use super::{
    addr::{FamilyId, NetlinkSocketAddr, PortNum},
    multicast_group::{GroupIdIter, GroupIdSet, MuilicastGroup, MAX_GROUPS},
    AnyNetlinkSocket,
};
use crate::{net::socket::netlink::addr::UNSPECIFIED_PORT, prelude::*, util::random::getrandom};

pub static NETLINK_SOCKET_TABLE: NetlinkSocketTable = NetlinkSocketTable::new();

/// All bound netlink sockets.
pub struct NetlinkSocketTable {
    families: RwMutex<BTreeMap<FamilyId, RwMutex<NetlinkFamily>>>,
}

impl NetlinkSocketTable {
    pub const fn new() -> Self {
        Self {
            families: RwMutex::new(BTreeMap::new()),
        }
    }

    /// Adds a new netlink family
    fn add_new_family(&self, family_id: FamilyId) {
        let mut families = self.families.write();
        if families.contains_key(&family_id) {
            return;
        }
        let new_family = RwMutex::new(NetlinkFamily::new(family_id));
        families.insert(family_id, new_family);
    }

    pub fn bind(
        &self,
        addr: &NetlinkSocketAddr,
        socket: Weak<dyn AnyNetlinkSocket>,
    ) -> Result<BoundHandle> {
        let families = self.families.read();

        let Some(family) = families.get(&addr.family_id()) else {
            return_errno_with_message!(Errno::EINVAL, "the netlink family does not exist")
        };

        let mut family = family.write();
        family.bind(addr, socket)
    }
}

/// Bound sockets of a single netlink family.
///
/// Each family has a unique `FamilyId`(u32).
/// Each family can have bound sockcts for unit cast
/// and at most 32 groups for multicast.
pub struct NetlinkFamily {
    id: FamilyId,
    unitcast_sockets: BTreeMap<PortNum, Weak<dyn AnyNetlinkSocket>>,
    multicast_groups: Box<[MuilicastGroup]>,
}

impl NetlinkFamily {
    /// Creates a new netlink family
    fn new(id: FamilyId) -> Self {
        let multicast_groups = (0u32..MAX_GROUPS)
            .map(|group_id| MuilicastGroup::new(group_id))
            .collect();
        Self {
            id,
            unitcast_sockets: BTreeMap::new(),
            multicast_groups,
        }
    }

    /// Binds a socket to the netlink family.
    /// Returns the bound addr.
    ///
    /// The socket will be bound to a port with `port_num`.
    /// If `port_num` is not provided, kernel will assign a port for it,
    /// typically, the port with the process id of current process.
    /// If the port is already used,
    /// this function will try to allocate a random unused port.
    ///
    /// Meanwhile, this socket can join one or more multicast groups,
    /// which is `specified` in groups.
    fn bind(
        &mut self,
        addr: &NetlinkSocketAddr,
        socket: Weak<dyn AnyNetlinkSocket>,
    ) -> Result<BoundHandle> {
        let port = if addr.port() != UNSPECIFIED_PORT {
            addr.port()
        } else {
            let mut random_port = current!().pid();
            while random_port == UNSPECIFIED_PORT
                || self.unitcast_sockets.contains_key(&random_port)
            {
                getrandom(random_port.as_bytes_mut()).unwrap();
            }
            random_port
        };

        if self.unitcast_sockets.contains_key(&port) {
            return_errno_with_message!(Errno::EADDRINUSE, "try to bind to an used port");
        }

        self.unitcast_sockets.insert(port, socket);

        for group_id in addr.groups().ids_iter() {
            debug_assert!(group_id < MAX_GROUPS);
            let group = &mut self.multicast_groups[group_id as usize];
            group.add_member(port);
        }

        Ok(BoundHandle::new(addr.family_id(), port, addr.groups()))
    }

    fn get_unicast_socket(&self, port: PortNum) -> Option<&Weak<dyn AnyNetlinkSocket>> {
        self.unitcast_sockets.get(&port)
    }

    fn get_multicast_groups<'a>(
        &'a self,
        iter: GroupIdIter<'a>,
    ) -> impl Iterator<Item = &'a MuilicastGroup> + 'a {
        iter.filter_map(|group_id| self.multicast_groups.get(group_id as usize))
    }
}

/// A bound netlink socket address.
///
/// When dropping a `BoundHandle`, the port will be automatically released.
#[derive(Debug)]
pub struct BoundHandle {
    family_id: FamilyId,
    port: PortNum,
    groups: GroupIdSet,
}

impl BoundHandle {
    fn new(family_id: FamilyId, port: PortNum, groups: GroupIdSet) -> Self {
        debug_assert_ne!(port, 0);

        Self {
            family_id,
            port,
            groups,
        }
    }

    const fn addr(&self) -> NetlinkSocketAddr {
        NetlinkSocketAddr::new(self.family_id, self.port, self.groups)
    }
}

impl Drop for BoundHandle {
    fn drop(&mut self) {
        let families = NETLINK_SOCKET_TABLE.families.read();
        let mut family = families.get(&self.family_id).unwrap().write();
        family.unitcast_sockets.remove(&self.port);

        for group_id in self.groups.ids_iter() {
            let group = &mut family.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }
    }
}

pub(super) fn init() {
    for family_id in 0..MAX_LINK {
        if is_standard_family_id(family_id) {
            NETLINK_SOCKET_TABLE.add_new_family(family_id);
        }
    }
}

/// Returns whether the `family` is a valid family id
pub fn is_valid_family_id(family_id: FamilyId) -> bool {
    family_id < MAX_LINK
}

/// Returns whether the `family` has reserved for some system use
pub fn is_standard_family_id(family_id: FamilyId) -> bool {
    StandardNetlinkFamily::try_from(family_id).is_ok()
}

/// These families are currently assigned for specific usage.
/// <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/netlink.h#L9>.
#[allow(non_camel_case_types)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum StandardNetlinkFamily {
    /// Routing/device hook
    NETLINK_ROUTE = 0,
    /// Unused number
    NETLINK_UNUSED = 1,
    /// Reserved for user mode socket protocols
    NETLINK_USERSOCK = 2,
    /// Unused number, formerly ip_queue
    NETLINK_FIREWALL = 3,
    /// socket monitoring
    NETLINK_SOCK_DIAG = 4,
    /// netfilter/iptables ULOG
    NETLINK_NFLOG = 5,
    /// ipsec
    NETLINK_XFRM = 6,
    /// SELinux event notifications
    NETLINK_SELINUX = 7,
    /// Open-iSCSI
    NETLINK_ISCSI = 8,
    /// auditing
    NETLINK_AUDIT = 9,
    NETLINK_FIB_LOOKUP = 10,
    NETLINK_CONNECTOR = 11,
    /// netfilter subsystem
    NETLINK_NETFILTER = 12,
    NETLINK_IP6_FW = 13,
    /// DECnet routing messages
    NETLINK_DNRTMSG = 14,
    /// Kernel messages to userspace
    NETLINK_KOBJECT_UEVENT = 15,
    NETLINK_GENERIC = 16,
    // leave room for NETLINK_DM (DM Events)
    /// SCSI Transports
    NETLINK_SCSITRANSPORT = 18,
    NETLINK_ECRYPTFS = 19,
    NETLINK_RDMA = 20,
    /// Crypto layer
    NETLINK_CRYPTO = 21,
    /// SMC monitoring
    NETLINK_SMC = 22,
}

const MAX_LINK: FamilyId = 32;
