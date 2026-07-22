// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    iface::InterfaceFlags,
    wire::{Ipv4Address, Ipv4Cidr},
};

use crate::{
    net::iface::{DEFAULT_TX_QUEUE_LEN, Iface, iter_all_ifaces},
    prelude::*,
    util::{
        ioctl::{InOutData, RawIoctl, dispatch_ioctl},
        net::CSocketAddrFamily,
    },
};

const IFNAMSIZ: usize = 16;
const IFREQ_DATA_SIZE: usize = 24;
const SOCKADDR_SIZE: usize = 16;

mod ioctl_defs {
    use super::{CIfConf, CIfReq};
    use crate::util::ioctl::{InOutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/sockios.h>.
    pub(super) type GetIfName       = ioc!(SIOCGIFNAME,     0x8910, InOutData<CIfReq>);
    pub(super) type GetIfConf       = ioc!(SIOCGIFCONF,     0x8912, InOutData<CIfConf>);
    pub(super) type GetIfFlags      = ioc!(SIOCGIFFLAGS,    0x8913, InOutData<CIfReq>);
    pub(super) type GetIfAddr       = ioc!(SIOCGIFADDR,     0x8915, InOutData<CIfReq>);
    pub(super) type GetIfDstAddr    = ioc!(SIOCGIFDSTADDR,  0x8917, InOutData<CIfReq>);
    pub(super) type GetIfBrdAddr    = ioc!(SIOCGIFBRDADDR,  0x8919, InOutData<CIfReq>);
    pub(super) type GetIfNetmask    = ioc!(SIOCGIFNETMASK,  0x891B, InOutData<CIfReq>);
    pub(super) type GetIfMetric     = ioc!(SIOCGIFMETRIC,   0x891D, InOutData<CIfReq>);
    pub(super) type GetIfMtu        = ioc!(SIOCGIFMTU,      0x8921, InOutData<CIfReq>);
    pub(super) type GetIfHwAddr     = ioc!(SIOCGIFHWADDR,   0x8927, InOutData<CIfReq>);
    pub(super) type GetIfIndex      = ioc!(SIOCGIFINDEX,    0x8933, InOutData<CIfReq>);
    pub(super) type GetIfTxQueueLen = ioc!(SIOCGIFTXQLEN,  0x8942, InOutData<CIfReq>);
    pub(super) type GetIfMap        = ioc!(SIOCGIFMAP,      0x8970, InOutData<CIfReq>);
}

pub(super) fn network_device_ioctl(raw_ioctl: RawIoctl) -> Result<i32> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        cmd @ GetIfName => {
            let mut ifreq = cmd.read()?;
            let iface = find_iface_by_index(ifreq.index())?;
            ifreq.set_name(iface.name())?;
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfConf => {
            let mut ifconf = cmd.read()?;
            get_ifconf(&mut ifconf)?;
            cmd.write(&ifconf)?;
            Ok(0)
        }
        cmd @ GetIfFlags => {
            with_iface(cmd, |ifreq, iface| {
                ifreq.set_flags(iface.flags());
                Ok(())
            })
        }
        cmd @ GetIfAddr => {
            with_iface(cmd, |ifreq, iface| {
                ifreq.set_sockaddr_ipv4(iface_ipv4_cidr(iface.as_ref())?.address());
                Ok(())
            })
        }
        cmd @ GetIfDstAddr => {
            with_iface(cmd, |ifreq, iface| {
                ifreq.set_sockaddr_ipv4(iface_ipv4_cidr(iface.as_ref())?.address());
                Ok(())
            })
        }
        cmd @ GetIfBrdAddr => {
            with_iface(cmd, |ifreq, iface| {
                let address = if iface.flags().contains(InterfaceFlags::BROADCAST) {
                    iface.broadcast_addr().unwrap_or(Ipv4Address::UNSPECIFIED)
                } else {
                    Ipv4Address::UNSPECIFIED
                };
                ifreq.set_sockaddr_ipv4(address);
                Ok(())
            })
        }
        cmd @ GetIfNetmask => {
            with_iface(cmd, |ifreq, iface| {
                ifreq.set_sockaddr_ipv4(iface_ipv4_cidr(iface.as_ref())?.netmask());
                Ok(())
            })
        }
        cmd @ GetIfMetric => {
            with_iface(cmd, |ifreq, _iface| {
                // Linux always reports zero because interface metrics are not implemented.
                ifreq.set_i32(0);
                Ok(())
            })
        }
        cmd @ GetIfMtu => {
            with_iface(cmd, |ifreq, iface| {
                let mtu = i32::try_from(iface.mtu())
                    .map_err(|_| Error::with_message(Errno::EOVERFLOW, "the MTU is too large"))?;
                ifreq.set_i32(mtu);
                Ok(())
            })
        }
        cmd @ GetIfHwAddr => {
            with_iface(cmd, |ifreq, iface| {
                ifreq.set_hardware_addr(iface.as_ref());
                Ok(())
            })
        }
        cmd @ GetIfIndex => {
            with_iface(cmd, |ifreq, iface| ifreq.set_index(iface.index()))
        }
        cmd @ GetIfTxQueueLen => {
            with_iface(cmd, |ifreq, _iface| {
                ifreq.set_i32(DEFAULT_TX_QUEUE_LEN as i32);
                Ok(())
            })
        }
        cmd @ GetIfMap => {
            with_iface(cmd, |ifreq, _iface| {
                // Asterinas does not expose legacy device memory, DMA, IRQ, or I/O port settings.
                ifreq.data = [0; IFREQ_DATA_SIZE];
                Ok(())
            })
        }
        _ => return_errno_with_message!(Errno::ENOTTY, "the socket ioctl command is unknown"),
    })
}

fn with_iface<const MAGIC: u8, const NR: u8, F>(
    cmd: crate::util::ioctl::Ioctl<MAGIC, NR, false, InOutData<CIfReq>>,
    op: F,
) -> Result<i32>
where
    F: FnOnce(&mut CIfReq, &Arc<Iface>) -> Result<()>,
{
    let mut ifreq = cmd.read()?;
    ifreq.terminate_name();
    let iface = find_iface_by_name(ifreq.name())?;
    op(&mut ifreq, iface)?;
    cmd.write(&ifreq)?;
    Ok(0)
}

fn get_ifconf(ifconf: &mut CIfConf) -> Result<()> {
    let ifreqs = iter_all_ifaces().filter_map(|iface| {
        let address = iface.ipv4_cidr()?.address();
        Some(CIfReq::from_name_and_addr(iface.name(), address))
    });

    if ifconf.buffer == 0 {
        let count = ifreqs.count();
        ifconf.len = i32::try_from(count * size_of::<CIfReq>()).unwrap();
        return Ok(());
    }

    let buffer_len = usize::try_from(ifconf.len).unwrap_or(0);
    let max_ifreqs = buffer_len / size_of::<CIfReq>();
    let task = ostd::task::Task::current().unwrap();
    let user_space = CurrentUserSpace::new(task.as_thread_local().unwrap());
    let mut writer = user_space.writer(ifconf.buffer, max_ifreqs * size_of::<CIfReq>())?;
    let mut count = 0;
    for ifreq in ifreqs.take(max_ifreqs) {
        writer.write_val(&ifreq)?;
        count += 1;
    }
    ifconf.len = i32::try_from(count * size_of::<CIfReq>()).unwrap();
    Ok(())
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CIfConf {
    len: i32,
    _padding: i32,
    buffer: Vaddr,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CIfReq {
    name: [u8; IFNAMSIZ],
    data: [u8; IFREQ_DATA_SIZE],
}

impl CIfReq {
    fn from_name_and_addr(name: &CStr, address: Ipv4Address) -> Self {
        let mut ifreq = Self {
            name: [0; IFNAMSIZ],
            data: [0; IFREQ_DATA_SIZE],
        };
        ifreq.set_name(name).unwrap();
        ifreq.set_sockaddr_ipv4(address);
        ifreq
    }

    fn terminate_name(&mut self) {
        self.name[IFNAMSIZ - 1] = 0;
    }

    fn name(&self) -> &CStr {
        CStr::from_bytes_until_nul(&self.name).unwrap()
    }

    fn set_name(&mut self, name: &CStr) -> Result<()> {
        let name = name.to_bytes_with_nul();
        if name.len() > IFNAMSIZ {
            return_errno_with_message!(Errno::ERANGE, "the interface name is too long");
        }

        self.name = [0; IFNAMSIZ];
        self.name[..name.len()].copy_from_slice(name);
        Ok(())
    }

    fn index(&self) -> i32 {
        i32::from_bytes(&self.data[..size_of::<i32>()])
    }

    fn set_index(&mut self, index: u32) -> Result<()> {
        let index = i32::try_from(index).map_err(|_| {
            Error::with_message(Errno::EOVERFLOW, "the interface index is too large")
        })?;
        self.set_i32(index);
        Ok(())
    }

    fn set_flags(&mut self, flags: InterfaceFlags) {
        let flags = flags.bits() as i16;
        self.data[..size_of::<i16>()].copy_from_slice(flags.as_bytes());
    }

    fn set_i32(&mut self, value: i32) {
        self.data[..size_of::<i32>()].copy_from_slice(value.as_bytes());
    }

    fn set_sockaddr_ipv4(&mut self, address: Ipv4Address) {
        self.data[..SOCKADDR_SIZE].fill(0);
        self.data[..size_of::<u16>()]
            .copy_from_slice((CSocketAddrFamily::AF_INET as u16).as_bytes());
        self.data[4..8].copy_from_slice(&address.octets());
    }

    fn set_hardware_addr(&mut self, iface: &Iface) {
        self.data[..SOCKADDR_SIZE].fill(0);
        self.data[..size_of::<u16>()].copy_from_slice((iface.type_() as u16).as_bytes());
        if let Some(address) = iface.ethernet_addr() {
            self.data[2..8].copy_from_slice(&address.0);
        }
    }
}

fn find_iface_by_name(name: &CStr) -> Result<&'static Arc<Iface>> {
    iter_all_ifaces()
        .find(|iface| iface.name() == name)
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "no interface found"))
}

fn find_iface_by_index(index: i32) -> Result<&'static Arc<Iface>> {
    iter_all_ifaces()
        .find(|iface| i32::try_from(iface.index()) == Ok(index))
        .ok_or_else(|| Error::with_message(Errno::ENODEV, "no interface found"))
}

fn iface_ipv4_cidr(iface: &Iface) -> Result<Ipv4Cidr> {
    iface
        .ipv4_cidr()
        .ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, "no IPv4 address found"))
}
