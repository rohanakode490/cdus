# CDUS - Cross-Device Unified System

A local-first system for syncing clipboards and transferring files between Linux/Windows/macOS and Android devices using libp2p and Noise encryption.

## Project Structure
- `crates/cdus-agent`: The core background daemon (Rust).
- `crates/cdus-ffi`: Android FFI bridge for the core logic.
- `src-tauri`: Desktop UI wrapper.
- `android`: Native Android application (Kotlin/Compose).
- `relay`: Optional signaling relay for remote connectivity.

---

## 1. Desktop Setup

### Build and Run the Agent
The agent must be running for the UI to work.
```bash
# Build and run the background daemon in interactive mode
cargo run -p cdus-agent
```

### Running as a Background Daemon/Service
You can install the agent to run automatically in the background on startup:

* **Linux (systemd user service):**
  ```bash
  # Install and start the service
  cargo run -p cdus-agent -- install

  # Stop and uninstall the service
  cargo run -p cdus-agent -- uninstall
  ```

* **macOS (launchd User Agent):**
  ```bash
  # Install and register the launchd plist (runs in user's graphical session)
  cargo run -p cdus-agent -- install

  # Unload and remove the launchd plist
  cargo run -p cdus-agent -- uninstall
  ```

* **Windows (Registry Startup Run Key):**
  ```bash
  # Install registry run key and spawn the daemon in user session
  cargo run -p cdus-agent -- install

  # Delete registry run key and terminate running processes
  cargo run -p cdus-agent -- uninstall
  ```

### Run the Desktop UI (Tauri)
```bash
# Install dependencies
npm install

# Run in development mode
npm run tauri dev
```

---

## 2. Android Setup

### Rebuilding the Rust FFI Core
If you change any code in `crates/`, you must rebuild the Android native libraries and bindings.

**Prerequisites:**
- `cargo ndk` installed (`cargo install cargo-ndk`)
- Android NDK configured in your environment.

**Build Commands:**
```bash
# 1. Build for ARM64 (Physical devices)
cargo ndk -t arm64-v8a build -p cdus-ffi --release
cp target/aarch64-linux-android/release/libcdus_ffi.so android/app/src/main/jniLibs/arm64-v8a/

# 2. Build for x86_64 (Emulators)
cargo ndk -t x86_64 build -p cdus-ffi --release
cp target/x86_64-linux-android/release/libcdus_ffi.so android/app/src/main/jniLibs/x86_64/

# 3. Generate Kotlin Bindings
cargo run --features=uniffi/cli --bin uniffi-bindgen generate \
    --library target/aarch64-linux-android/release/libcdus_ffi.so \
    --language kotlin \
    --out-dir android/app/src/main/java/
```

### Install on Device
```bash
cd android
./gradlew installDebug
```

---

## 3. Usage & Troubleshooting

### First-Time Pairing
1. Ensure both devices are on the same WiFi network.
2. Open CDUS on both devices.
3. On Desktop, click **"Scan for Devices"**.
4. Select your phone and verify the 4-digit PIN on both screens.
5. Once paired, the devices will show as **Online**.

### Handling "Unknown" Node IDs
If a device shows an `unknown` ID in the list, it means it was paired with an older version of the app.
1. Click **Unpair** on both Desktop and Android.
2. Re-pair the devices to exchange the new secure Node IDs.

### File Transfers
- Use the **"Send File"** button in the **Devices** tab next to a specific online device.
- Received files on Android are automatically moved to the public **Downloads** folder.
