use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "PascalCase")]
pub enum AdapterKind {
    Ethernet,
    #[serde(rename = "Wi-Fi")]
    Wifi,
    #[serde(rename = "USB tether")]
    UsbTether,
    Mobile,
    Virtual,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdapterInfo {
    pub id: String,
    pub name: String,
    pub description: String,
    pub kind: AdapterKind,
    pub ipv4: Vec<String>,
    pub ipv6: Vec<String>,
    pub gateways: Vec<String>,
    pub link_speed_bps: u64,
    pub connected: bool,
    pub interface_index: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectedAdapter {
    pub id: String,
    pub name: String,
    pub local_ip: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartRequest {
    pub url: String,
    pub destination_dir: String,
    pub adapters: Vec<SelectedAdapter>,
    pub overwrite: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum DownloadStatus {
    Waiting,
    Probing,
    Downloading,
    Paused,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DownloadSnapshot {
    pub id: String,
    pub url: String,
    pub filename: String,
    pub destination: String,
    pub status: DownloadStatus,
    pub downloaded: u64,
    pub total: Option<u64>,
    pub bytes_per_second: f64,
    pub eta_seconds: Option<f64>,
    pub range_supported: Option<bool>,
    pub error: Option<String>,
    pub interface_speeds: HashMap<String, f64>,
    #[serde(default)]
    pub interface_bytes: HashMap<String, u64>,
    pub checksum_sha256: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEvent {
    pub timestamp: String,
    pub level: String,
    pub download_id: Option<String>,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Settings {
    pub connections_per_interface: usize,
    pub chunk_size_mib: u64,
    pub retry_count: usize,
    pub download_directory: String,
    pub bandwidth_limit_mbps: f64,
    pub auto_use_new_adapters: bool,
    pub start_minimized: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            connections_per_interface: 3,
            chunk_size_mib: 8,
            retry_count: 4,
            download_directory: dirs::download_dir()
                .unwrap_or_else(|| std::env::current_dir().unwrap_or_default())
                .to_string_lossy()
                .into_owned(),
            bandwidth_limit_mbps: 0.0,
            auto_use_new_adapters: true,
            start_minimized: false,
        }
    }
}

impl Settings {
    pub fn validate(&self) -> Result<(), String> {
        if !(1..=8).contains(&self.connections_per_interface) {
            return Err("Connections per adapter must be between 1 and 8".into());
        }
        if !(1..=128).contains(&self.chunk_size_mib) {
            return Err("Chunk size must be between 1 and 128 MiB".into());
        }
        if self.retry_count > 20 {
            return Err("Retry count must not exceed 20".into());
        }
        if self.bandwidth_limit_mbps < 0.0 {
            return Err("Bandwidth limit cannot be negative".into());
        }
        Ok(())
    }
}
