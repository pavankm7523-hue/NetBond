# NetBond installation guide

## Requirements

- 64-bit Windows 11 (Windows 10 is expected to work with WebView2)
- Microsoft Edge WebView2 Runtime
- Two or more independent Internet connections for multi-network use

No administrator account or network driver is required.

## Recommended setup

1. Open the [latest release](https://github.com/pavankm7523-hue/NetBond/releases/latest).
2. Download `NetBond_0.1.5_x64-setup.exe`.
3. Double-click it and finish the per-user installation.
4. Launch **NetBond** from the Start menu.

The current build is not code-signed, so Windows SmartScreen may show **Windows protected your PC**. Verify the SHA-256 checksum first. If it matches and you trust this repository, choose **More info**, then **Run anyway**.

## MSI or portable options

- For MSI, download `NetBond_0.1.5_x64_en-US.msi` and follow Windows Installer.
- For portable use, download `NetBond.exe`, move it to a permanent folder, and run it directly.

## Verify the download

Download `SHA256SUMS.txt`, open PowerShell in your Downloads folder, and run:

```powershell
Get-FileHash .\NetBond_0.1.5_x64-setup.exe -Algorithm SHA256
Get-Content .\SHA256SUMS.txt
```

The values must match exactly.

## First launch

1. Connect Wi-Fi, Ethernet, USB tethering, or the networks you plan to use.
2. Open NetBond and wait for adapter cards to appear.
3. Select online adapters with usable IP addresses.
4. For a file, paste its direct HTTP/HTTPS URL and choose a destination.
5. For BitTorrent, open **BitTorrent** and load a legal `.torrent` or magnet URI.
6. Start and check that intended interfaces show real application traffic.

For extra speed, use separate Internet paths, such as broadband plus phone tethering. Two adapters on the same router usually share one bottleneck.

## Uninstall

For installed builds, open **Settings → Apps → Installed apps**, find **NetBond**, and choose **Uninstall**. For portable use, close the app and delete `NetBond.exe`.

## Troubleshooting

- **No adapters:** reconnect them, confirm Windows has Internet access, and restart NetBond.
- **Only one adapter has traffic:** confirm both links independently reach the Internet. VPN or Windows routing policy may override the path.
- **HTTP is not faster:** the server may not support ranges or may rate-limit clients.
- **Torrent waits for metadata:** use a legal torrent with active trackers or peers.
- **App does not open:** install or repair Microsoft Edge WebView2 Runtime.
- **SmartScreen warning:** the release is unsigned; verify its checksum before running.

Open a [GitHub issue](https://github.com/pavankm7523-hue/NetBond/issues) if needed. Remove private URLs, magnets, and IP addresses before posting.
