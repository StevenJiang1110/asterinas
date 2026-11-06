// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(route = crate::net::socket::netlink::route)]

use core::net::{IpAddr, Ipv4Addr, Ipv6Addr};

use crate::{
    net::socket::netlink::message::{Attribute, CAttrHeader, ContinueAttrRead},
    prelude::*,
    util::MultiRead,
};

/// Route attributes.
#[expect(clippy::upper_case_acronyms)]
#[expect(non_camel_case_types)]
#[repr(u16)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub(in route) enum RouteAttrClass {
    UNSPEC = 0,
    DST = 1,
    SRC = 2,
    IIF = 3,
    OIF = 4,
    GATEWAY = 5,
    PRIORITY = 6,
    PREFSRC = 7,
    METRICS = 8,
    MULTIPATH = 9,
    PROTOINFO = 10,
    FLOW = 11,
    CACHEINFO = 12,
    SESSION = 13,
    MP_ALGO = 14,
    TABLE = 15,
    MARK = 16,
    MFC_STATS = 17,
    VIA = 18,
    NEWDST = 19,
    PREF = 20,
    ENCAP_TYPE = 21,
    ENCAP = 22,
    EXPIRES = 23,
    PAD = 24,
    UID = 25,
    TTL_PROPAGATE = 26,
    IP_PROTO = 27,
    SPORT = 28,
    DPORT = 29,
    NH_ID = 30,
    FLOWLABEL = 31,
}

#[derive(Debug)]
pub(crate) enum RouteAttr {
    DestinationPort(u16),
    Dst(IpAddr),
    FlowLabel(u32),
    Gateway(IpAddr),
    Iif(u32),
    IpProtocol(u8),
    Mark(u32),
    Oif(u32),
    PrefSrc(IpAddr),
    Priority(u32),
    SourcePort(u16),
    Src(IpAddr),
    Table(u32),
    Uid(u32),
}

impl RouteAttr {
    pub(in route) fn class(&self) -> RouteAttrClass {
        match self {
            RouteAttr::DestinationPort(_) => RouteAttrClass::DPORT,
            RouteAttr::Dst(_) => RouteAttrClass::DST,
            RouteAttr::FlowLabel(_) => RouteAttrClass::FLOWLABEL,
            RouteAttr::Gateway(_) => RouteAttrClass::GATEWAY,
            RouteAttr::Iif(_) => RouteAttrClass::IIF,
            RouteAttr::IpProtocol(_) => RouteAttrClass::IP_PROTO,
            RouteAttr::Mark(_) => RouteAttrClass::MARK,
            RouteAttr::Oif(_) => RouteAttrClass::OIF,
            RouteAttr::PrefSrc(_) => RouteAttrClass::PREFSRC,
            RouteAttr::Priority(_) => RouteAttrClass::PRIORITY,
            RouteAttr::SourcePort(_) => RouteAttrClass::SPORT,
            RouteAttr::Src(_) => RouteAttrClass::SRC,
            RouteAttr::Table(_) => RouteAttrClass::TABLE,
            RouteAttr::Uid(_) => RouteAttrClass::UID,
        }
    }
}

impl Attribute for RouteAttr {
    fn type_(&self) -> u16 {
        self.class() as u16
    }

    fn payload_as_bytes(&self) -> &[u8] {
        match self {
            RouteAttr::DestinationPort(value) => value.as_bytes(),
            RouteAttr::Dst(value) => value.as_octets(),
            RouteAttr::FlowLabel(value) => value.as_bytes(),
            RouteAttr::Gateway(value) => value.as_octets(),
            RouteAttr::Iif(value) => value.as_bytes(),
            RouteAttr::IpProtocol(value) => value.as_bytes(),
            RouteAttr::Mark(value) => value.as_bytes(),
            RouteAttr::Oif(value) => value.as_bytes(),
            RouteAttr::PrefSrc(value) => value.as_octets(),
            RouteAttr::Priority(value) => value.as_bytes(),
            RouteAttr::SourcePort(value) => value.as_bytes(),
            RouteAttr::Src(value) => value.as_octets(),
            RouteAttr::Table(value) => value.as_bytes(),
            RouteAttr::Uid(value) => value.as_bytes(),
        }
    }

    fn read_from(header: &CAttrHeader, reader: &mut dyn MultiRead) -> Result<ContinueAttrRead<Self>>
    where
        Self: Sized,
    {
        let payload_len = header.payload_len();

        let Ok(class) = RouteAttrClass::try_from(header.type_()) else {
            reader.skip_some(payload_len);
            return Ok(ContinueAttrRead::SkippedUnsupported);
        };

        let res = match (class, payload_len) {
            (RouteAttrClass::FLOWLABEL, 4) => {
                Self::FlowLabel(reader.read_val_opt::<u32>()?.unwrap())
            }
            (RouteAttrClass::DPORT, 2) => {
                Self::DestinationPort(reader.read_val_opt::<u16>()?.unwrap())
            }
            (RouteAttrClass::SPORT, 2) => Self::SourcePort(reader.read_val_opt::<u16>()?.unwrap()),
            (RouteAttrClass::IP_PROTO, 1) => {
                Self::IpProtocol(reader.read_val_opt::<u8>()?.unwrap())
            }
            (RouteAttrClass::UID, 4) => Self::Uid(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::MARK, 4) => Self::Mark(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::DST, 4) => {
                Self::Dst(Ipv4Addr::from(reader.read_val_opt::<[u8; 4]>()?.unwrap()).into())
            }
            (RouteAttrClass::DST, 16) => {
                Self::Dst(Ipv6Addr::from(reader.read_val_opt::<[u8; 16]>()?.unwrap()).into())
            }
            (RouteAttrClass::GATEWAY, 4) => {
                Self::Gateway(Ipv4Addr::from(reader.read_val_opt::<[u8; 4]>()?.unwrap()).into())
            }
            (RouteAttrClass::GATEWAY, 16) => {
                Self::Gateway(Ipv6Addr::from(reader.read_val_opt::<[u8; 16]>()?.unwrap()).into())
            }
            (RouteAttrClass::IIF, 4) => Self::Iif(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::OIF, 4) => Self::Oif(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::PREFSRC, 4) => {
                Self::PrefSrc(Ipv4Addr::from(reader.read_val_opt::<[u8; 4]>()?.unwrap()).into())
            }
            (RouteAttrClass::PREFSRC, 16) => {
                Self::PrefSrc(Ipv6Addr::from(reader.read_val_opt::<[u8; 16]>()?.unwrap()).into())
            }
            (RouteAttrClass::PRIORITY, 4) => Self::Priority(reader.read_val_opt::<u32>()?.unwrap()),
            (RouteAttrClass::SRC, 4) => {
                Self::Src(Ipv4Addr::from(reader.read_val_opt::<[u8; 4]>()?.unwrap()).into())
            }
            (RouteAttrClass::SRC, 16) => {
                Self::Src(Ipv6Addr::from(reader.read_val_opt::<[u8; 16]>()?.unwrap()).into())
            }
            (RouteAttrClass::TABLE, 4) => Self::Table(reader.read_val_opt::<u32>()?.unwrap()),

            (
                RouteAttrClass::DST
                | RouteAttrClass::GATEWAY
                | RouteAttrClass::IIF
                | RouteAttrClass::OIF
                | RouteAttrClass::PREFSRC
                | RouteAttrClass::PRIORITY
                | RouteAttrClass::SRC
                | RouteAttrClass::MARK
                | RouteAttrClass::UID
                | RouteAttrClass::IP_PROTO
                | RouteAttrClass::SPORT
                | RouteAttrClass::DPORT
                | RouteAttrClass::FLOWLABEL
                | RouteAttrClass::TABLE,
                _,
            ) => {
                warn!("route attribute `{:?}` contains invalid payload", class);
                reader.skip_some(payload_len);
                return Ok(ContinueAttrRead::skipped_invalid(
                    Errno::ERANGE,
                    "the route attribute is invalid",
                ));
            }

            (_, _) => {
                warn!("route attribute `{:?}` is not supported", class);
                reader.skip_some(payload_len);
                return Ok(ContinueAttrRead::SkippedUnsupported);
            }
        };

        Ok(ContinueAttrRead::Parsed(res))
    }
}
