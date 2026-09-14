# Diagnostic & Benchmarking Scripts

This directory contains manual diagnostic, benchmarking, and regression verification scripts for CDUS.

## Script Inventory

- `check_agent.py`: Healthcheck script to poll and ping the local UNIX domain socket daemon.
- `reproduce_bug_9.py`: Regression harness verifying agent socket creation and clean shutdown.
- `run_benchmark.py`: Initiates 1GB network throughput benchmark between two local/remote nodes.
- `test_ipc.py`: General CLI utility for manual IPC command testing (`ping`, `pair`, `send_file`, `listen`).
- `test_resume.py`: Validates chunked file transfer resumption across process restarts and simulated crashes.
- `verify_transfer.py`: Automated end-to-end 5MB transfer and SHA256 integrity verification.
