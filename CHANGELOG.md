# Changelog

## 0.1.5 — 2026-09-26

- Make torrent percentage and progress prominent and add expandable per-file progress.
- Show downloaded bytes, live speed, and percentage contribution for each Internet adapter.
- Recognize USB NCM phone tethering as `USB tether`.
- Apply adapter selection changes to the live peer-source pool so new peer connections can use newly connected interfaces.

## 0.1.4 — 2026-09-26

- Allow rqbit's dual-stack UDP tracker client to fall back to an IPv4 socket when selected Windows adapters have IPv4 source addresses only.

## 0.1.3 — 2026-09-26

- Hide Windows tunnel, WAN miniport, hotspot-internal, kernel-debug, and other ghost interfaces from the Network dashboard.

## 0.1.2 — 2026-09-26

- Ignore stale Windows hidden-adapter indexes instead of aborting network discovery.

## 0.1.1 — 2026-09-26

- Allow `.torrent` and magnet metadata review before selecting a download adapter.
- Keep metadata inspection separate from the long-lived, source-bound torrent session.
- Refresh and validate the selected adapters when the torrent download actually starts.

## 0.1.0 — 2026-09-25

- Initial public Windows release.
- Multi-interface HTTP/HTTPS range downloading.
- BitTorrent `.torrent` and magnet support using `librqbit`, trackers, and DHT.
- Source-bound torrent peer distribution across selected online adapters.
- Real per-interface application traffic and aggregate bandwidth dashboard.
- Pause, resume, cancellation, retries, recovery, SHA-256, and atomic finalization.
- NSIS, MSI, and portable x64 packages.
