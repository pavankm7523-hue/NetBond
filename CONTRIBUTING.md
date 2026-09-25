# Contributing to NetBond

Search existing issues first. Never post private URLs, magnet links, credentials, or personal IP addresses. Keep BitTorrent examples limited to content that is legal to redistribute.

1. Fork and clone the repository on Windows.
2. Install the prerequisites in the README.
3. Create a focused branch and change.
4. Run `.\scripts\test.ps1`.
5. In the pull request, explain the visible change, tests, and routing limitations.

Networking changes should test adapter loss, fallback, or byte accounting where applicable. UI changes should remain accessible and preserve the white-and-orange design system.
