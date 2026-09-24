#!/bin/bash
# sync-ffi.sh - Automates CDUS FFI synchronization between Rust and Android

set -e

PROJECT_ROOT=$(pwd)
FFI_CRATE_DIR="$PROJECT_ROOT/crates/cdus-ffi"
ANDROID_DIR="$PROJECT_ROOT/android"
ANDROID_JNI_DIR="$ANDROID_DIR/app/src/main/jniLibs"
KOTLIN_OUT_DIR="$ANDROID_DIR/app/src/main/java"
TARGET_DIR="$PROJECT_ROOT/target"

echo "🚀 Starting FFI synchronization..."

# 1. Build the Rust FFI library for the host (to run bindgen)
echo "📦 Building FFI crate for host..."
cd "$FFI_CRATE_DIR"
cargo build -p cdus-ffi

# 2. Identify the host library
HOST_LIB="$TARGET_DIR/debug/libcdus_ffi.so"
if [ ! -f "$HOST_LIB" ]; then
    # Handle macOS extension if needed in future
    HOST_LIB="$TARGET_DIR/debug/libcdus_ffi.dylib"
fi

# 3. Generate Kotlin Bindings
echo "🧬 Generating Kotlin bindings..."
cargo run --bin uniffi-bindgen generate --library "$HOST_LIB" --language kotlin --out-dir "$KOTLIN_OUT_DIR"

# 4. Attempt to build for Android targets if NDK is available
# Note: This step might fail if local environment isn't fully set up for cross-compilation,
# but we'll try to sync what we have.
echo "🛠️ Attempting to build native Android libraries..."
if [ -z "$ANDROID_NDK_HOME" ] && [ -d "$ANDROID_HOME/ndk" ]; then
    LATEST_NDK=$(ls -1 "$ANDROID_HOME/ndk" 2>/dev/null | sort -V | tail -n 1)
    if [ -n "$LATEST_NDK" ]; then
        export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$LATEST_NDK"
    fi
fi

if command -v cargo-ndk &> /dev/null; then
    cargo ndk -t arm64-v8a -t x86_64 build --release -p cdus-ffi
    
    echo "Syncing arm64-v8a..."
    mkdir -p "$ANDROID_JNI_DIR/arm64-v8a"
    cp "$TARGET_DIR/aarch64-linux-android/release/libcdus_ffi.so" "$ANDROID_JNI_DIR/arm64-v8a/"
    rm -f "$ANDROID_JNI_DIR/arm64-v8a"/libif_watch*.so
    
    echo "Syncing x86_64..."
    mkdir -p "$ANDROID_JNI_DIR/x86_64"
    cp "$TARGET_DIR/x86_64-linux-android/release/libcdus_ffi.so" "$ANDROID_JNI_DIR/x86_64/"
    rm -f "$ANDROID_JNI_DIR/x86_64"/libif_watch*.so
else
    echo "⚠️ cargo-ndk not found. Falling back to syncing host library (debug only)."
    echo "   Note: This may only work on matching emulator architectures."
    
    mkdir -p "$ANDROID_JNI_DIR/arm64-v8a" "$ANDROID_JNI_DIR/x86_64"
    cp "$HOST_LIB" "$ANDROID_JNI_DIR/arm64-v8a/"
    cp "$HOST_LIB" "$ANDROID_JNI_DIR/x86_64/"
fi

echo "✅ FFI synchronization complete!"
echo "   - Kotlin bindings updated in: $KOTLIN_OUT_DIR"
echo "   - Native libraries synced to: $ANDROID_JNI_DIR"
