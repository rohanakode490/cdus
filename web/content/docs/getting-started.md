---
title: "Getting Started with CDUS"
description: "Install and configure CDUS across Linux, macOS, Windows, and Android for instant encrypted peer-to-peer clipboard and file synchronization."
order: 1
section: "Quick Start"
publishedAt: 2026-06-01
---

# Getting Started with CDUS

CDUS connects your personal computers and mobile devices into a single secure local network. It requires no centralized user account, no cloud storage subscription, and no open inbound router ports.

## System Prerequisites

- **Desktop Systems:** Linux (Ubuntu 22.04+, Debian 12+, Arch, Fedora), Windows 10/11 (64-bit), or macOS 12+ (Apple Silicon & Intel).
- **Mobile Devices:** Android 10+ (API level 29 or higher).
- **Network Environment:** Local Area Network (Wi-Fi or Ethernet) for direct P2P connections. When firewalls block direct communication, CDUS automatically uses an encrypted fallback relay.

## 1. Installation

### Linux
Download the native `.deb` package or universal `.AppImage`:
```bash
# Debian / Ubuntu
sudo dpkg -i cdus-desktop_0.1.0_amd64.deb

# AppImage
chmod +x cdus-desktop_0.1.0_amd64.AppImage
./cdus-desktop_0.1.0_amd64.AppImage
```

### Windows
Run the signed NSIS installer `cdus-desktop_0.1.0_x64-setup.exe` or deploy via `.msi` in enterprise environments.

### macOS
Open `cdus-desktop_0.1.0_universal.dmg` and drag the application icon to your `/Applications` directory.

### Android
Install the native APK from GitHub Releases or deploy through your mobile device management tool.

## 2. Pairing Your First Two Devices

1. Launch CDUS on your primary computer.
2. Navigate to the **Pairing** tab and click **Generate Pairing Code**. A secure single-use QR code will appear on screen.
3. Open CDUS on your Android phone or second computer, tap **Scan QR**, and point the camera at your desktop screen.
4. Both devices execute an authenticated **Noise XX** cryptographic handshake. Once verified, the devices establish an encrypted session and exchange device certificates.

## 3. Verifying Synchronization

- **Clipboard:** Copy text or an image on your laptop. The content appears immediately in the clipboard history of your phone.
- **File Transfer:** Drag any file into the CDUS window or drop zone. Files transfer directly over LAN sockets with BLAKE3 chunk hashing for verified integrity.
