use network_interface::{NetworkInterface, NetworkInterfaceConfig};
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    sync::{Mutex, OnceLock},
};

pub fn index_for(ip: IpAddr) -> io::Result<u32> {
    NetworkInterface::show()
        .map_err(io::Error::other)?
        .into_iter()
        .find(|nic| nic.addr.iter().any(|a| a.ip() == ip))
        .map(|nic| nic.index)
        .filter(|index| *index != 0)
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::AddrNotAvailable,
                "Selected adapter address is unavailable",
            )
        })
}

pub fn is_online(ip: IpAddr) -> bool {
    let Ok(index) = index_for(ip) else {
        return false;
    };
    #[cfg(windows)]
    {
        use windows_sys::Win32::NetworkManagement::IpHelper::{GetIfEntry2, MIB_IF_ROW2};
        let mut row = MIB_IF_ROW2 {
            InterfaceIndex: index,
            ..Default::default()
        };
        // SAFETY: the API receives a correctly sized, initialized row.
        return unsafe { GetIfEntry2(&mut row) } == 0 && row.OperStatus == 1;
    }
    #[cfg(not(windows))]
    {
        true
    }
}

fn peer_routes() -> &'static Mutex<HashMap<SocketAddr, IpAddr>> {
    static ROUTES: OnceLock<Mutex<HashMap<SocketAddr, IpAddr>>> = OnceLock::new();
    ROUTES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn selected_sources() -> &'static Mutex<Option<Vec<IpAddr>>> {
    static SOURCES: OnceLock<Mutex<Option<Vec<IpAddr>>>> = OnceLock::new();
    SOURCES.get_or_init(|| Mutex::new(None))
}

/// Replace the live source pool used for new torrent peer connections.
/// Existing sockets remain on their original interfaces.
pub fn set_selected_sources(sources: Vec<IpAddr>) {
    if let Ok(mut selected) = selected_sources().lock() {
        *selected = Some(sources);
    }
}

pub fn current_selected_sources() -> Option<Vec<IpAddr>> {
    selected_sources().lock().ok()?.clone()
}

pub fn record_peer(peer: SocketAddr, source: IpAddr) {
    if let Ok(mut routes) = peer_routes().lock() {
        routes.insert(peer, source);
    }
}

pub fn peer_source(peer: SocketAddr) -> Option<IpAddr> {
    peer_routes().lock().ok()?.get(&peer).copied()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remembers_the_source_interface_for_a_peer() {
        let peer: SocketAddr = "198.51.100.8:6881".parse().unwrap();
        let source: IpAddr = "192.0.2.10".parse().unwrap();
        record_peer(peer, source);
        assert_eq!(peer_source(peer), Some(source));
    }
}

/// Enforce an outgoing interface in addition to source-address binding.
/// IPv4's index is network byte order; IPv6's is host byte order.
pub fn enforce(socket: &socket2::Socket, ip: IpAddr) -> io::Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawSocket;
        use windows_sys::Win32::Networking::WinSock::{
            setsockopt, WSAGetLastError, IPPROTO_IP, IPPROTO_IPV6, IPV6_UNICAST_IF, IP_UNICAST_IF,
            SOCKET_ERROR,
        };
        let index = index_for(ip)?;
        let (level, option, value) = if ip.is_ipv4() {
            (IPPROTO_IP, IP_UNICAST_IF, index.to_be())
        } else {
            (IPPROTO_IPV6, IPV6_UNICAST_IF, index)
        };
        // SAFETY: socket is live, value is a DWORD and the buffer length matches.
        let result = unsafe {
            setsockopt(
                socket.as_raw_socket() as usize,
                level,
                option,
                (&value as *const u32).cast(),
                std::mem::size_of::<u32>() as i32,
            )
        };
        if result == SOCKET_ERROR {
            return Err(io::Error::from_raw_os_error(unsafe { WSAGetLastError() }));
        }
    }
    #[cfg(not(windows))]
    {
        let _ = (socket, ip);
    }
    Ok(())
}
