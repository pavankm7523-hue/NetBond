use crate::models::{AdapterInfo, AdapterKind};
use serde::Deserialize;
use std::process::Command;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "PascalCase")]
struct RawAdapter {
    id: String,
    name: String,
    description: String,
    status: String,
    interface_index: u32,
    link_speed_bps: u64,
    #[serde(default)]
    ipv4: Vec<String>,
    #[serde(default)]
    ipv6: Vec<String>,
    #[serde(default)]
    gateways: Vec<String>,
}

pub fn enumerate() -> Result<Vec<AdapterInfo>, String> {
    #[cfg(not(target_os = "windows"))]
    return Ok(Vec::new());

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // Get-NetAdapter/Get-NetIPConfiguration are non-elevated projections over
        // Windows' Network Adapter and IP Helper APIs. Arrays are forced to keep
        // the JSON shape stable for adapters with one or zero addresses.
        let script = r#"
$ErrorActionPreference='Stop'
[Console]::OutputEncoding = [System.Text.UTF8Encoding]::new($false)
$ipByIndex = @{}
# Query valid IP configurations once. Calling Get-NetIPConfiguration for each
# hidden adapter can fail when Windows keeps a stale adapter whose ifIndex no
# longer exists in MSFT_NetIPInterface (common after USB tether/VPN removal).
try {
  @(Get-NetIPConfiguration -ErrorAction SilentlyContinue) | ForEach-Object {
    $ipByIndex[[uint32]$_.InterfaceIndex] = $_
  }
} catch {
  # Adapter discovery remains useful even if Windows cannot currently provide
  # address details. Such adapters are returned without IPs and are not usable.
}
$items = @(Get-NetAdapter -IncludeHidden | Where-Object { $_.InterfaceDescription -notmatch 'Loopback' } | ForEach-Object {
  $a = $_
  $ip = $ipByIndex[[uint32]$a.ifIndex]
  [PSCustomObject]@{
    Id = $a.InterfaceGuid.ToString()
    Name = $a.Name
    Description = [string]$a.InterfaceDescription
    Status = $a.Status.ToString()
    InterfaceIndex = [uint32]$a.ifIndex
    LinkSpeedBps = [uint64]$a.TransmitLinkSpeed
    Ipv4 = @($ip.IPv4Address | Where-Object { $_ -and $_.IPAddress } | ForEach-Object { $_.IPAddress })
    Ipv6 = @($ip.IPv6Address | Where-Object { $_ -and $_.IPAddress } | ForEach-Object { $_.IPAddress })
    Gateways = @($ip.IPv4DefaultGateway,$ip.IPv6DefaultGateway | Where-Object { $_ -and $_.NextHop } | ForEach-Object { $_.NextHop })
  }
})
ConvertTo-Json -InputObject $items -Compress -Depth 5
"#;
        let output = Command::new("powershell.exe")
            .creation_flags(0x08000000)
            .args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                script,
            ])
            .output()
            .map_err(|e| format!("Could not launch Windows network query: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "Windows network query failed: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        let raw: Vec<RawAdapter> = serde_json::from_slice(&output.stdout)
            .map_err(|e| format!("Invalid adapter data: {e}"))?;
        Ok(raw
            .into_iter()
            .map(|a| {
                let hay = format!("{} {}", a.name, a.description).to_lowercase();
                let kind = if hay.contains("wi-fi")
                    || hay.contains("wifi")
                    || hay.contains("wireless")
                {
                    AdapterKind::Wifi
                } else if hay.contains("usb")
                    && (hay.contains("rndis")
                        || hay.contains("tether")
                        || hay.contains("remote ndis"))
                {
                    AdapterKind::UsbTether
                } else if hay.contains("mobile") || hay.contains("cellular") || hay.contains("wwan")
                {
                    AdapterKind::Mobile
                } else if hay.contains("hyper-v")
                    || hay.contains("virtual")
                    || hay.contains("vpn")
                    || hay.contains("loopback")
                {
                    AdapterKind::Virtual
                } else if hay.contains("ethernet") || hay.contains("gbe") || hay.contains("lan") {
                    AdapterKind::Ethernet
                } else {
                    AdapterKind::Other
                };
                AdapterInfo {
                    id: a.id,
                    name: a.name,
                    description: a.description,
                    kind,
                    ipv4: a.ipv4,
                    ipv6: a.ipv6,
                    gateways: a.gateways,
                    link_speed_bps: a.link_speed_bps,
                    connected: a.status.eq_ignore_ascii_case("up"),
                    interface_index: a.interface_index,
                }
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn kind_serialization_matches_frontend() {
        assert_eq!(
            serde_json::to_string(&AdapterKind::Wifi).unwrap(),
            "\"Wi-Fi\""
        );
        assert_eq!(
            serde_json::to_string(&AdapterKind::UsbTether).unwrap(),
            "\"USB tether\""
        );
    }
}
