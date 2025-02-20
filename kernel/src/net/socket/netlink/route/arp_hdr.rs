// SPDX-License-Identifier: MPL-2.0

//! Global definitions for the ARP (RFC 826) protocol.

use crate::prelude::*;

#[repr(u16)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum DeviceType {
    // Arp protocol hardware identifiers

    /// from KA9Q: NET/ROM pseudo
    NETROM = 0,
    /// Ethernet 10Mbps
    ETHER = 1,
    /// Experimental Ethernet
    EETHER = 2,

    // Dummy types for non ARP hardware

    /// IPIP tunnel
    TUNNEL = 768,
    /// IP6IP6 tunnel
    TUNNEL6 = 769,
    /// Frame Relay Access Device
    FRAD = 770,
    /// SKIP vif
    SKIP = 771,
    /// Loopback device
    LOOPBACK = 772,
    /// Localtalk device
    LOCALTALK = 773,

    // TODO: This enum is not exhaustive
}

