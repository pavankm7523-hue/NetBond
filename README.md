# NetBond

[![CI](https://github.com/pavankm7523-hue/NetBond/actions/workflows/ci.yml/badge.svg)](https://github.com/pavankm7523-hue/NetBond/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/pavankm7523-hue/NetBond)](https://github.com/pavankm7523-hue/NetBond/releases/latest)
[![License: MIT](https://img.shields.io/badge/License-MIT-orange.svg)](LICENSE)

NetBond is a modern Windows 11 download manager that can use multiple selected network adapters for HTTP(S) range downloads and BitTorrent peer connections. Its dashboard shows real application traffic per interface so you can see whether Wi-Fi, Ethernet, or USB tethering is actually contributing.

> NetBond does not create a true bonded network link and does not install a network driver. Faster downloads require independent working Internet paths, a server or swarm capable of supplying enough data, and multiple connections that Windows can route through the selected adapters.

## Download

Get the latest version from [GitHub Releases](https://github.com/pavankm7523-hue/NetBond/releases/latest):

- `NetBond_0.1.6_x64-setup.exe` — recommended per-user Windows installer
- `NetBond_0.1.6_x64_en-US.msi` — MSI installer
- `NetBond.exe` — portable build; no installation required
- `SHA256SUMS.txt` — checksums for verifying downloads

See [INSTALLATION.md](INSTALLATION.md) for the complete Windows installation and first-run guide.

## Features

- Clean white-and-orange Windows desktop interface
- Live adapter discovery with status, type, addresses, gateways, and link speed
- Real per-interface application download/upload traffic and live graphs
- Aggregate speed, downloaded data, progress, and ETA
- HTTP(S) byte-range scheduling across selected adapters
- Source-IP binding plus Windows `IP_UNICAST_IF` interface selection
- Pause, resume, cancel, retry, crash recovery, and SHA-256 verification
- Safe single-connection fallback when a server does not support byte ranges
- `.torrent` files and magnet URIs through the mature `librqbit` engine
- Torrent metadata preview and individual file selection
- Peer count, download/upload speed, progress, ETA, upload limit, and seeding controls
- Torrent peer connections distributed across selected online adapters
- Graceful adapter disconnect/reconnect handling
- Detailed connection, range, retry, and routing logs

## Using multiple networks

Connect two or more independent links—for example Ethernet, Wi-Fi, and USB phone tethering—then select them in NetBond before starting a download.

For HTTP(S), NetBond splits a range-capable file into chunks and binds workers to selected interfaces. For torrents, each outgoing peer connection is assigned to an available selected interface. The dashboard reports bytes from the application's actual transfers; Windows-wide adapter counters are shown separately.

If all adapters share the same router, ISP, VPN, or bottleneck, speed may not increase. A torrent also needs enough peers. NetBond never labels traffic as bonded unless distinct connections are visibly using multiple adapters.

## BitTorrent quick start

1. Open **BitTorrent** in NetBond.
2. Choose a legal `.torrent` file or paste a magnet URI.
3. Wait for metadata, review the name, size, and file list, and select the files you want.
4. Select at least two online adapters before starting the first torrent session.
5. Set the peer limit, upload limit, and seeding preference.
6. Start the torrent and watch per-interface peer traffic in real time.

The adapter set is established when the torrent session starts. Restart NetBond to use a different set. NetBond contains no links to copyrighted or pirated material; use BitTorrent only for content you are authorized to download and share.

## How it works

HTTP workers bind sockets to an adapter's source address and apply its Windows interface index before connecting. Torrent connections use a patched dual-stack connector that rotates outgoing peers across selected, currently online source adapters. Per-interface torrent totals come from real `librqbit` peer byte counters matched to the recorded source adapter.

Read [ARCHITECTURE.md](ARCHITECTURE.md) for the networking model, accuracy boundaries, security behavior, and limitations.

## Build from source

Requirements: Windows 11, Node.js 20+, Rust stable MSVC, Visual Studio 2022 Build Tools with **Desktop development with C++**, and WebView2.

```powershell
git clone https://github.com/pavankm7523-hue/NetBond.git
cd NetBond
npm ci
.\scripts\dev.ps1
```

Build installers and the portable executable:

```powershell
.\scripts\build.ps1
```

Run the complete test suite:

```powershell
.\scripts\test.ps1
```

## Project layout

- `src/` — TypeScript dashboard and torrent interface
- `src-tauri/src/` — Rust download, adapter, traffic, settings, and torrent engines
- `src-tauri/vendor/` — upstream crates patched for Windows source routing
- `scripts/` — development, test, icon, and release helpers
- `.github/` — CI and contribution templates

## Current limitations

- Windows only; current builds are unsigned x64 binaries.
- Internet speed is not the sum of adapter link-rate labels.
- HTTP acceleration requires valid server byte-range support.
- Torrent performance depends on swarm health and peer behavior.
- Incoming torrent listening is disabled; peer connections are outgoing.
- The physical release test machine had one live Wi-Fi adapter, so multi-WAN behavior is covered by routing/unit tests but should also be validated on your own two-link setup.

## Safety and privacy

NetBond runs as the signed-in user, does not require administrator rights at runtime, does not change Windows routes, and does not install a driver. Download logs and recovery metadata remain local. Report security issues using [SECURITY.md](SECURITY.md).

## Contributing and license

Contributions are welcome—see [CONTRIBUTING.md](CONTRIBUTING.md). NetBond is under the [MIT License](LICENSE). Modified upstream components retain their original licenses; see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
