# CDUS Desktop E2E Testing Scaffolding

This directory contains the End-to-End (E2E) automated testing setup for the CDUS Desktop (Tauri) application across Windows, macOS, and Linux using [`tauri-driver`](https://v2.tauri.app/develop/tests/webdriver/).

---

## Architecture Overview

`tauri-driver` acts as a translation layer between standard W3C WebDriver test suites and platform-native WebViews:

```
[ Test Runner (Node.js / Bun) ]
             │ (W3C WebDriver HTTP)
             ▼
      [ tauri-driver ] (Port 4444)
             │
     ┌───────┼──────────────────┐
     ▼       ▼                  ▼
[WebKitWebDriver]  [msedgedriver.exe]   [safaridriver]
  (Linux GTK)       (Windows WebView2)    (macOS WKWebView)
     │       │                  │
     ▼       ▼                  ▼
[   CDUS Desktop Application Window   ]
```

---

## Platform Prerequisites

### 1. Install `tauri-driver`
```bash
cargo install tauri-driver
```

### 2. Platform WebDrivers
- **Windows**: Microsoft Edge WebDriver is preinstalled on Windows 10/11 and `windows-latest` runners. If missing, download matching your Edge version from [Microsoft Edge Developer](https://developer.microsoft.com/en-us/microsoft-edge/tools/webdriver/).
- **macOS**: Safari WebDriver is preinstalled on macOS. Enable it once with:
  ```bash
  safaridriver --enable
  ```
- **Linux (Ubuntu/Debian)**:
  ```bash
  sudo apt-get install -y webkit2gtk-driver
  ```

---

## Running E2E Tests Locally

1. **Build the desktop binary in release or debug mode:**
   ```bash
   bun run tauri build --no-bundle
   ```

2. **Launch `tauri-driver` in the background:**
   ```bash
   tauri-driver &
   ```

3. **Execute the test suite with the path to the built binary:**
   ```bash
   # On Linux:
   TAURI_BIN_PATH="../../src-tauri/target/release/tauri-app" bun run test:e2e

   # On Windows:
   set TAURI_BIN_PATH=..\..\src-tauri\target\release\tauri-app.exe
   bun run test:e2e

   # On macOS:
   TAURI_BIN_PATH="../../src-tauri/target/release/bundle/macos/tauri-app.app/Contents/MacOS/tauri-app" bun run test:e2e
   ```
