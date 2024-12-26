// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;

#[derive(Debug, Clone, Copy, CopyGetters, Setters)]
#[get_copy = "pub"]
#[set = "pub"]
pub struct TcpOptionSet {
    no_delay: bool,
    maxseg: u32,
    keep_idle: u32,
    defer_accept: Retrans,
    window_clamp: u32,
    congestion: CongestionControl,
}

pub const DEFAULT_MAXSEG: u32 = 536;
pub const DEFAULT_KEEP_IDLE: u32 = 7200;
pub const DEFAULT_WINDOW_CLAMP: u32 = 0x8000_0000;

impl TcpOptionSet {
    pub fn new() -> Self {
        Self {
            no_delay: false,
            maxseg: DEFAULT_MAXSEG,
            keep_idle: DEFAULT_KEEP_IDLE,
            defer_accept: Retrans(0),
            window_clamp: DEFAULT_WINDOW_CLAMP,
            congestion: CongestionControl::Reno,
        }
    }
}

impl Default for TcpOptionSet {
    fn default() -> Self {
        Self::new()
    }
}

const HZ: u32 = 100;
/// Initial RTO value
const TCP_TIMEOUT_INIT: u32 = HZ;
const TCP_RTO_MAX: u32 = 120 * HZ;

/// The number of retransmits
#[derive(Debug, Clone, Copy)]
pub struct Retrans(u8);

impl Retrans {
    /// Converts seconds to retransmits
    pub const fn from_secs(seconds: u32) -> Self {
        if seconds == 0 {
            return Self(0);
        }

        let mut timeout = TCP_TIMEOUT_INIT / HZ;
        let rto_max = TCP_RTO_MAX / HZ;
        let mut period = timeout;
        let mut res = 1;

        while seconds > period && res < 255 {
            res += 1;
            timeout <<= 1;
            if timeout > rto_max {
                timeout = rto_max;
            }
            period += timeout;
        }

        Self(res)
    }

    /// Converts retransmits to seconds
    pub const fn to_secs(self) -> u32 {
        let mut retrans = self.0;

        if retrans == 0 {
            return 0;
        }

        let mut timeout = TCP_TIMEOUT_INIT / HZ;
        let rto_max = TCP_RTO_MAX / HZ;
        let mut period = timeout;

        while retrans > 1 {
            retrans -= 1;
            timeout <<= 1;
            if timeout > rto_max {
                timeout = rto_max;
            }
            period += timeout;
        }

        period
    }
}

#[derive(Debug, Clone, Copy)]
pub enum CongestionControl {
    Reno,
    Cubic,
}

impl CongestionControl {
    const RENO: &'static str = "reno";
    const CUBIC: &'static str = "cubic";

    pub fn new(name: &str) -> Result<Self> {
        let congestion = match name {
            Self::RENO => Self::Reno,
            Self::CUBIC => Self::Cubic,
            _ => return_errno_with_message!(Errno::EINVAL, "unsupported congestion name"),
        };

        Ok(congestion)
    }

    pub fn name(&self) -> &'static str {
        match self {
            Self::Reno => Self::RENO,
            Self::Cubic => Self::CUBIC,
        }
    }
}
