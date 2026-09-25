export type Adapter = {
  id: string;
  name: string;
  description: string;
  kind: "Ethernet" | "Wi-Fi" | "USB tether" | "Mobile" | "Virtual" | "Other";
  ipv4: string[];
  ipv6: string[];
  gateways: string[];
  link_speed_bps: number;
  connected: boolean;
  interface_index: number;
};

export type DownloadStatus = "waiting" | "probing" | "downloading" | "paused" | "completed" | "failed" | "cancelled";

export type DownloadSnapshot = {
  id: string;
  url: string;
  filename: string;
  destination: string;
  status: DownloadStatus;
  downloaded: number;
  total: number | null;
  bytes_per_second: number;
  eta_seconds: number | null;
  range_supported: boolean | null;
  error: string | null;
  interface_speeds: Record<string, number>;
  interface_bytes: Record<string, number>;
  checksum_sha256: string | null;
};

export type LogEvent = {
  timestamp: string;
  level: "info" | "warn" | "error";
  download_id?: string;
  message: string;
};

export type Settings = {
  connections_per_interface: number;
  chunk_size_mib: number;
  retry_count: number;
  download_directory: string;
  bandwidth_limit_mbps: number;
  auto_use_new_adapters: boolean;
  start_minimized: boolean;
};
