---
title: "Pairing & Cryptographic Security"
description: "Comprehensive breakdown of Noise XX pairing handshakes, Noise IK reconnection, BLAKE3 integrity verification, and key storage."
order: 3
section: "Security"
publishedAt: 2026-06-01
---

# Pairing & Cryptographic Security

CDUS treats security as a foundational primitive rather than an afterthought. All data transferred between paired devices is end-to-end encrypted with authenticated forward secrecy.

## The Noise Protocol Handshakes

CDUS implements two distinct handshake patterns from the formal **Noise Protocol Framework**:

### 1. Initial Device Pairing (Noise XX)
When introducing two previously untrusted devices:
- Neither device knows the other's static public key in advance.
- An out-of-band QR code transmits ephemeral connection metadata and a visual confirmation digest.
- The devices execute a 3-message `Noise_XX_25519_ChaChaPoly_BLAKE2s` handshake:
  - Ephemeral key exchange
  - Mutual static public key transmission
  - Authenticated identity confirmation
- Both devices display matching 6-digit confirmation pins derived from the cryptographic handshake hash to prevent Man-in-the-Middle (MitM) attacks.

### 2. Fast Reconnection (Noise IK)
When reconnecting with a previously paired device:
- The initiator already knows the responder's static public key.
- The connection completes in a single round-trip `Noise_IK_25519_ChaChaPoly_BLAKE2s` handshake.
- Ephemeral keys generate fresh session keys with forward secrecy for every connection cycle.

## Data Integrity via BLAKE3

Files are partitioned into 1MB chunks prior to transmission. Each chunk includes:
- A progressive chunk index
- A 32-byte BLAKE3 cryptographic hash
- The complete file root BLAKE3 hash

The receiving daemon verifies each chunk against its hash immediately upon arrival on disk. Corrupted or interrupted chunks are re-requested independently without re-transmitting the entire file.

## Key Storage

- **Desktop (Linux/macOS/Windows):** Static private keys reside in the operating system's native keychain (libsecret on Linux, macOS Keychain, and Windows Credential Manager).
- **Android:** Keys are stored in the hardware-backed **Android KeyStore** with StrongBox or TEE isolation when available.
