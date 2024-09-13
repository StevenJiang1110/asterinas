use log::debug;

use ostd::arch::qemu::{exit_qemu, QemuExitCode};
use smoltcp::iface::{Config, Interface, SocketSet};
use smoltcp::phy::{Device, Loopback, Medium};
use smoltcp::socket::tcp;
use smoltcp::time::Instant;
use smoltcp::wire::{EthernetAddress, IpAddress, IpCidr};

use crate::time::clocks::MonotonicClock;

use crate::prelude::*;

pub fn test_smoltcp_bandwidth() {
    let mut device = Loopback::new(Medium::Ethernet);

    // Create interface
    let config = match device.capabilities().medium {
        Medium::Ethernet => {
            Config::new(EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]).into())
        }
        Medium::Ip => Config::new(smoltcp::wire::HardwareAddress::Ip),     
    };

    let clock = MonotonicClock::get();
    let mut iface = Interface::new(config, &mut device, instant_now(clock));
    iface.update_ip_addrs(|ip_addrs| {
        ip_addrs
            .push(IpCidr::new(IpAddress::v4(127, 0, 0, 1), 8))
            .unwrap();
    });

    // Create sockets
    let server_socket = {
        let tcp_rx_buffer = tcp::SocketBuffer::new(vec![0; 65536]);
        let tcp_tx_buffer = tcp::SocketBuffer::new(vec![0; 65536]);
        tcp::Socket::new(tcp_rx_buffer, tcp_tx_buffer)
    };

    let client_socket = {
        let tcp_rx_buffer = tcp::SocketBuffer::new(vec![0; 65536]);
        let tcp_tx_buffer = tcp::SocketBuffer::new(vec![0; 65536]);
        tcp::Socket::new(tcp_rx_buffer, tcp_tx_buffer)
    };

    let mut sockets: [_; 2] = Default::default();
    let mut sockets = SocketSet::new(&mut sockets[..]);
    let server_handle = sockets.add(server_socket);
    let client_handle = sockets.add(client_socket);

    let start_time = clock.read_time();

    let mut did_listen = false;
    let mut did_connect = false;
    let mut processed = 0;
    while processed < 1024 * 1024 * 1024 {
        iface.poll(instant_now(clock), &mut device, &mut sockets);

        let socket = sockets.get_mut::<tcp::Socket>(server_handle);
        if !socket.is_active() && !socket.is_listening() && !did_listen {
            debug!("listening");
            socket.listen(1234).unwrap();
            did_listen = true;
        }

        while socket.can_recv() {
            let received = socket.recv(|buffer| (buffer.len(), buffer.len())).unwrap();
            debug!("got {:?}", received,);
            processed += received;
        }

        let socket = sockets.get_mut::<tcp::Socket>(client_handle);
        let cx = iface.context();
        if !socket.is_open() && !did_connect {
            debug!("connecting");
            socket
                .connect(cx, (IpAddress::v4(127, 0, 0, 1), 1234), 65000)
                .unwrap();
            did_connect = true;
        }

        while socket.can_send() {
            debug!("sending");
            socket.send(|buffer| (buffer.len(), ())).unwrap();
        }
    }

    let duration = clock.read_time() - start_time;
    println!(
        "done in {} s, bandwidth is {} Gbps",
        duration.as_millis() as f64 / 1000.0,
        (processed as u64 * 8 / duration.as_millis() as u64) as f64 / 1000000.0
    );

    exit_qemu(QemuExitCode::Success)
}

fn instant_now(clock: &Arc<MonotonicClock>) -> Instant {
    Instant::from_micros(clock.read_time().as_micros() as i64)
}
