---
title: "CDUS Architecture & Core Engine"
description: "Deep dive into the native Rust sync engine, thread-pipelined architecture, bounded Flume channels, and resilient network transport."
order: 2
section: "Core System"
publishedAt: 2026-06-01
---

# CDUS Architecture & Core Engine

CDUS is intentionally engineered without heavy asynchronous runtimes like Tokio to maintain absolute predictability, zero thread-pool starvation, and minimal memory footprint on resource-constrained mobile and embedded environments.

## Design Philosophy

- **Thread-Pipelined Concurrency:** Long-running system stages run as dedicated OS threads communicating across bounded **Flume** channels.
- **Protocol Stability:** All wire communications and internal IPC messages serialize using **MessagePack** for fast packing and unpacking.
- **Zero Heavy Async:** Disk I/O, socket polling, and cryptographic operations run on dedicated pipelines, preventing transfer bottlenecks from stalling real-time clipboard notifications.

## System Pipeline Diagram

```text
[Network Inbound Socket]
          │
          ▼
   (Decrypt & Verify) ──> Noise XX / IK Session Decryptor
          │
          ▼
   [Inbound Message Dispatcher]
          ├───────────────┬───────────────┐
          ▼               ▼               ▼
   (Clipboard Queue) (File Chunk Pipeline) (Heartbeat / PEX)
          │               │               │
          ▼               ▼               ▼
     OS Clipboard     Disk Writer       Routing Table
```

## Transport Architecture

CDUS employs a dual-path networking engine:

1. **Direct Peer-to-Peer (LAN & PEX):** Discovers local peers using mDNS UDP broadcasts and Peer Exchange (PEX). Once paired, peers stream data directly over encrypted TCP/WebSocket connections.
2. **Relay Fallback (TURN-based):** When peers reside on restrictive carrier-grade NATs (CGNAT) or corporate networks, CDUS transparently routes encrypted chunks through a lightweight Go relay. Chunks remain end-to-end encrypted; the relay forwards raw bytes without decrypting payloads.
