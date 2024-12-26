// SPDX-License-Identifier: MPL-2.0

use super::RawSocketOption;
use crate::{
    impl_raw_socket_option,
    net::socket::ip::stream::options::{
        Congestion, DeferAccept, KeepIdle, MaxSegment, NoDelay, WindowClamp,
    },
    prelude::*,
    util::net::options::SocketOption,
};

/// Sock options for tcp socket.
///
/// The raw definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/tcp.h#L92
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
pub enum CTcpOptionName {
    /// Turn off Nagle's algorithm.
    NODELAY = 1,
    /// Limit MSS     
    MAXSEG = 2,
    /// Never send partially complete segments      
    CORK = 3,
    /// Start keeplives after this period      
    KEEPIDLE = 4,
    /// Interval between keepalives  
    KEEPINTVL = 5,
    /// Wake up listener only when data arrive   
    DEFER_ACCEPT = 9,
    /// Bound advertised window
    WINDOW_CLAMP = 10,
    /// Congestion control algorithm
    CONGESTION = 13,
}

pub fn new_tcp_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CTcpOptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CTcpOptionName::NODELAY => Ok(Box::new(NoDelay::new())),
        CTcpOptionName::MAXSEG => Ok(Box::new(MaxSegment::new())),
        CTcpOptionName::KEEPIDLE => Ok(Box::new(KeepIdle::new())),
        CTcpOptionName::DEFER_ACCEPT => Ok(Box::new(DeferAccept::new())),
        CTcpOptionName::WINDOW_CLAMP => Ok(Box::new(WindowClamp::new())),
        CTcpOptionName::CONGESTION => Ok(Box::new(Congestion::new())),
        _ => return_errno_with_message!(Errno::ENOPROTOOPT, "unsupported tcp-level option"),
    }
}

impl_raw_socket_option!(NoDelay);
impl_raw_socket_option!(MaxSegment);
impl_raw_socket_option!(KeepIdle);
impl_raw_socket_option!(DeferAccept);
impl_raw_socket_option!(WindowClamp);
impl_raw_socket_option!(Congestion);
