# NetBond architecture

## Networking contract

NetBond creates a separate HTTP client for each selected adapter. Each Windows socket is bound to the adapter's local address and configured with `IP_UNICAST_IF` using its current interface index before `connect`; TLS is layered over that socket. Proxies are disabled for these direct source-bound connections, and HTTPS redirects cannot downgrade to HTTP.

Binding a source IP selects the adapter that owns that address, but it cannot create an Internet path that Windows does not have. The adapter must have a usable route to the destination. Two adapters behind the same router normally share the same WAN bottleneck and will not provide additive Internet capacity. NetBond does not alter route metrics, install a driver, or require administrator rights.

References:

- [Microsoft Winsock `bind`](https://learn.microsoft.com/windows/win32/api/winsock/nf-winsock-bind)
- [Microsoft `GetAdaptersAddresses`](https://learn.microsoft.com/windows/win32/api/iphlpapi/nf-iphlpapi-getadaptersaddresses)
- [Microsoft `IP_UNICAST_IF`](https://learn.microsoft.com/windows/win32/winsock/ipproto-ip-socket-options)

## Components

```text
Tauri/WebView UI
  ├─ adapter selector + network dashboard
  ├─ queue, progress, settings and log views
  ├─ BitTorrent metadata, file selection and peer statistics
  └─ typed commands/events
              │
Rust application core
  ├─ Windows adapter inventory
  ├─ Windows interface octet counters (`GetIfEntry2`)
  ├─ URL probe (redirects, size, Range capability)
  ├─ dynamic chunk scheduler
  │    └─ N workers per adapter, each source-IP bound
  ├─ exact offset writer + range validation
  ├─ pause/cancel/retry + restart sidecars
  └─ SHA-256 + recoverable atomic finalization

BitTorrent engine (librqbit)
  ├─ .torrent and tracker-backed magnet metadata
  ├─ per-file selection and peer connection limits
  ├─ pause/resume, upload limit and optional seeding
  └─ round-robin peer sockets across all selected, online Windows adapters
```

The scheduler uses small ranges rather than one fixed region per adapter. A fast interface naturally claims more ranges, which rebalances work without moving bytes already in flight. A response is accepted only when it is HTTP 206, its `Content-Range` exactly matches the requested interval, and its body length is exact.

The partial file has a hidden, unique `.part` name. Completed chunk state is written through a temporary sidecar and renamed into place. At completion NetBond hashes the partial file, temporarily backs up an explicitly replaceable destination, then renames the completed partial file. A failed final rename restores the original.

## Windows adapter inventory

The current implementation uses non-elevated `Get-NetAdapter` and `Get-NetIPConfiguration`, Windows PowerShell projections over the Network Adapter/IP Helper stack. It reports stable interface GUID, friendly name, interface index, operational state, unicast addresses, gateways and transmit link speed. USB RNDIS/tether descriptions are classified heuristically; users can always identify them by name and IP.

## Recovery and lifecycle

- Adapter inventory is compared every five seconds, including after sleep/wake.
- Worker connection errors use bounded exponential retry.
- Interrupted in-flight chunks return to pending when a saved download is recovered.
- Sidecars in the configured download folder load as paused after restart.
- Existing destination files are never touched unless the user checks the replacement option.

## Deliberate limits

- HTTP/1.1 and HTTP/2 are supported. HTTP/3 is intentionally not enabled because source binding behavior needs a separately audited QUIC path.
- Servers without byte ranges fall back to one source-bound connection.
- The dashboard separates NetBond payload-byte counters from Windows adapter counters. Windows counters include every application and protocol overhead; NetBond counters include application payload and retries.
- Torrent peer connections are assigned round-robin to selected adapters. Each TCP peer socket is source-bound and configured with `IP_UNICAST_IF`. Per-interface torrent totals come from librqbit's actual per-peer payload counters joined to the source adapter recorded when that peer socket connected.
- DHT is enabled for trackerless magnet discovery. Local-service discovery is disabled; outgoing peer transfers are distributed across the selected adapters.
- A server/CDN may throttle by account, IP, token, or aggregate request rate, limiting any gain.
