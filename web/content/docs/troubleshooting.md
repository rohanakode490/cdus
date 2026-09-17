---
title: "CDUS Network Troubleshooting Guide"
description: "Diagnose direct peer-to-peer discovery, firewall configuration, relay fallbacks, and connection status badges in CDUS."
order: 4
section: "Operations"
publishedAt: 2026-06-01
---

# CDUS Network Troubleshooting Guide

Use this guide to diagnose connection failures, verify network paths, and inspect system log outputs.

## Connection Status Badges

CDUS displays a real-time status badge next to each paired device:

| Status Badge | Color | Meaning | Recovery Action |
| :--- | :--- | :--- | :--- |
| **LAN** | Green (`#4CAF50`) | Direct peer-to-peer connection over local Wi-Fi or Ethernet. Optimal transfer speeds. | None required. |
| **Relay** | Orange (`#EF6C00`) | Direct P2P blocked by firewall/NAT. Streaming via fallback relay. | Check router client isolation settings. |
| **Offline** | Red (`#EF4444`) | Peer unreachable or daemon stopped. | Verify target device is powered on and running CDUS. |

## Diagnosing Local Peer Discovery

CDUS uses UDP port `5353` for mDNS local discovery and TCP port `4242` for direct data transfers.

### Linux Firewall (UFW)
```bash
sudo ufw allow 4242/tcp comment "CDUS Direct P2P"
sudo ufw allow 5353/udp comment "CDUS mDNS Discovery"
```

### Windows Firewall
Ensure `cdus-desktop.exe` has "Private Networks" permission granted in Windows Defender Firewall settings.

### Wi-Fi AP Client Isolation
Many public or guest Wi-Fi networks enable **Client Isolation**, blocking devices on the same subnet from talking to one another.
- When client isolation is enabled, CDUS automatically switches from direct LAN to the encrypted fallback relay.
- If you prefer direct LAN speed, switch to a trusted private network or a mobile hotspot without AP isolation.

## Inspecting Logs

To inspect live diagnostics from the native daemon:

```bash
# Linux
journalctl --user -u cdus-agent -f

# Or run the binary directly with debug logging
RUST_LOG=cdus=debug cdus-agent
```
