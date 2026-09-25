#[cfg(test)]
pub(crate) mod tests;

use crate::Error;
#[cfg(windows)]
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::{ffi::CString, net::IpAddr, num::NonZeroU32, str::FromStr};

#[derive(Debug, Clone)]
pub struct BindDevice {
    #[allow(unused)]
    index: NonZeroU32,
    #[allow(unused)]
    name: CString,
    #[cfg(windows)]
    sources: Arc<Vec<(IpAddr, NonZeroU32)>>,
    #[cfg(windows)]
    next: Arc<AtomicUsize>,
}

impl BindDevice {
    #[cfg(not(windows))]
    pub fn new_from_name(name: &str) -> crate::Result<Self> {
        let name = CString::new(name).map_err(|_| Error::BindDeviceInvalid)?;

        let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
        let index = NonZeroU32::new(index)
            .ok_or_else(|| Error::BindDeviceInvalidError(std::io::Error::last_os_error()))?;
        Ok(Self { index, name })
    }

    #[cfg(windows)]
    pub fn new_from_name(name: &str) -> crate::Result<Self> {
        // Resolve the selected source address to its Windows interface index.
        let sources = name
            .split(';')
            .map(|value| {
                let ip = value.parse().map_err(|_| Error::BindDeviceInvalid)?;
                let index = netbond_route::index_for(ip).map_err(Error::BindDeviceInvalidError)?;
                Ok((ip, NonZeroU32::new(index).ok_or(Error::BindDeviceInvalid)?))
            })
            .collect::<crate::Result<Vec<_>>>()?;
        let index = sources
            .first()
            .map(|s| s.1)
            .ok_or(Error::BindDeviceInvalid)?;
        let name = CString::new(name).map_err(|_| Error::BindDeviceInvalid)?;
        Ok(Self {
            index,
            name,
            sources: Arc::new(sources),
            next: Arc::new(AtomicUsize::new(0)),
        })
    }

    pub fn index(&self) -> NonZeroU32 {
        self.index
    }

    #[cfg(windows)]
    pub fn next_source(&self, is_v6: bool) -> crate::Result<IpAddr> {
        let start = self.next.fetch_add(1, Ordering::Relaxed);
        for offset in 0..self.sources.len() {
            let ip = self.sources[(start + offset) % self.sources.len()].0;
            if ip.is_ipv6() == is_v6 && netbond_route::is_online(ip) {
                return Ok(ip);
            }
        }
        Err(Error::BindDeviceInvalid)
    }

    #[cfg(windows)]
    pub fn bind_source(
        &self,
        sref: &socket2::Socket,
        ip: IpAddr,
        is_v6: bool,
    ) -> crate::Result<()> {
        if ip.is_ipv6() != is_v6 {
            return Err(Error::BindDeviceInvalid);
        }
        netbond_route::enforce(sref, ip).map_err(Error::BindDeviceSetDeviceError)
    }

    pub fn name(&self) -> &str {
        // We constructed from a string so this can't fail
        unsafe { std::str::from_utf8_unchecked(self.name.to_bytes()) }
    }

    #[cfg(target_os = "macos")]
    pub fn bind_sref(&self, sref: &socket2::Socket, is_v6: bool) -> crate::Result<()> {
        if is_v6 {
            sref.bind_device_by_index_v6(Some(self.index))
                .map_err(Error::BindDeviceSetDeviceError)
        } else {
            sref.bind_device_by_index_v4(Some(self.index))
                .map_err(Error::BindDeviceSetDeviceError)
        }
    }

    #[cfg(not(any(target_os = "macos", windows)))]
    pub fn bind_sref(&self, sref: &socket2::Socket, _is_v6: bool) -> crate::Result<()> {
        let name = self.name.as_bytes_with_nul();
        sref.bind_device(Some(name))
            .map_err(Error::BindDeviceSetDeviceError)
    }

    #[cfg(windows)]
    pub fn bind_sref(&self, sref: &socket2::Socket, is_v6: bool) -> crate::Result<()> {
        let ip = self.next_source(is_v6)?;
        self.bind_source(sref, ip, is_v6)
    }
}

impl FromStr for BindDevice {
    type Err = crate::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new_from_name(s)
    }
}

#[cfg(all(test, windows))]
mod netbond_tests {
    use super::*;
    use network_interface::{NetworkInterface, NetworkInterfaceConfig};

    #[test]
    fn rotates_across_available_ipv4_sources() {
        let mut ips = NetworkInterface::show()
            .unwrap()
            .into_iter()
            .flat_map(|nic| nic.addr.into_iter().map(|addr| addr.ip()))
            .filter(|ip| ip.is_ipv4() && netbond_route::is_online(*ip))
            .collect::<Vec<_>>();
        ips.sort();
        ips.dedup();
        if ips.len() < 2 {
            return;
        }
        let device = BindDevice::new_from_name(&format!("{};{}", ips[0], ips[1])).unwrap();
        assert_eq!(device.next_source(false).unwrap(), ips[0]);
        assert_eq!(device.next_source(false).unwrap(), ips[1]);
    }
}
