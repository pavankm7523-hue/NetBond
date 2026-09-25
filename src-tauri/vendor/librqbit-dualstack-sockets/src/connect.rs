use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr};

use socket2::SockRef;

use crate::{Error, bind_device::BindDevice};

#[derive(Clone, Copy, Debug, Default)]
pub struct ConnectOpts<'a> {
    pub source_port: Option<u16>,
    pub bind_device: Option<&'a BindDevice>,
}

pub async fn tcp_connect<'a>(
    addr: SocketAddr,
    opts: ConnectOpts<'a>,
) -> crate::Result<tokio::net::TcpStream> {
    let (sock, bind_addr) = if addr.is_ipv6() {
        (
            tokio::net::TcpSocket::new_v6().map_err(Error::SocketNew)?,
            SocketAddr::from((Ipv6Addr::UNSPECIFIED, opts.source_port.unwrap_or(0))),
        )
    } else {
        (
            tokio::net::TcpSocket::new_v4().map_err(Error::SocketNew)?,
            SocketAddr::from((Ipv4Addr::UNSPECIFIED, opts.source_port.unwrap_or(0))),
        )
    };
    let sref = SockRef::from(&sock);
    #[cfg(windows)]
    let selected = opts
        .bind_device
        .map(|bd| bd.next_source(addr.is_ipv6()))
        .transpose()?;
    #[cfg(windows)]
    let bind_addr = selected
        .map(|ip| SocketAddr::new(ip, bind_addr.port()))
        .unwrap_or(bind_addr);

    if let Some(bd) = opts.bind_device {
        #[cfg(windows)]
        bd.bind_source(
            &sref,
            selected.ok_or(Error::BindDeviceInvalid)?,
            addr.is_ipv6(),
        )?;
        #[cfg(not(windows))]
        bd.bind_sref(&sref, addr.is_ipv6())?;
    }

    if bind_addr.port() > 0 || (cfg!(windows) && opts.bind_device.is_some()) {
        #[cfg(not(windows))]
        sref.set_reuse_port(true).map_err(Error::ReusePort)?;
        sref.set_reuse_address(true).map_err(Error::ReuseAddress)?;
        sref.bind(&bind_addr.into()).map_err(Error::Bind)?;
    }

    let stream = sock.connect(addr).await.map_err(Error::Connect)?;
    #[cfg(windows)]
    if let Some(source) = selected {
        netbond_route::record_peer(addr, source);
    }
    Ok(stream)
}
