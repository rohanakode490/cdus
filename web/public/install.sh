#!/usr/bin/env sh
set -e

# ==============================================================================
# CDUS Installer Script
# https://github.com/rohanakode490/cdus
# ==============================================================================

REPO="rohanakode490/cdus"
COLOR_RESET="\033[0m"
COLOR_BOLD="\033[1m"
COLOR_CYAN="\033[36m"
COLOR_GREEN="\033[32m"
COLOR_RED="\033[31m"
COLOR_YELLOW="\033[33m"

print_step() {
  printf "${COLOR_CYAN}${COLOR_BOLD}==>${COLOR_RESET} ${COLOR_BOLD}%s${COLOR_RESET}\n" "$1"
}

print_success() {
  printf "${COLOR_GREEN}${COLOR_BOLD}✓${COLOR_RESET} %s\n" "$1"
}

print_warning() {
  printf "${COLOR_YELLOW}${COLOR_BOLD}!${COLOR_RESET} %s\n" "$1"
}

print_error() {
  printf "${COLOR_RED}${COLOR_BOLD}✗ Error:${COLOR_RESET} %s\n" "$1" >&2
}

# 1. Platform Detection
OS=$(uname -s)
ARCH=$(uname -m)

if [ "$OS" != "Linux" ]; then
  print_error "This automated shell script currently supports Linux."
  if [ "$OS" = "Darwin" ]; then
    printf "For macOS (Apple Silicon / Intel), please download the official .dmg from:\n"
  else
    printf "For Windows, macOS, or Android, please download the official release from:\n"
  fi
  printf "  https://github.com/%s/releases/latest\n" "$REPO"
  exit 1
fi

case "$ARCH" in
  x86_64|amd64)
    ARCH_NAME="x86_64"
    ;;
  aarch64|arm64)
    ARCH_NAME="aarch64"
    ;;
  *)
    print_error "Unsupported Linux architecture: $ARCH"
    exit 1
    ;;
esac

# 2. Dependency verification
if ! command -v curl >/dev/null 2>&1 && ! command -v wget >/dev/null 2>&1; then
  print_error "Either 'curl' or 'wget' is required to download CDUS."
  exit 1
fi

download_file() {
  url="$1"
  dest="$2"
  if command -v curl >/dev/null 2>&1; then
    curl -fL --progress-bar "$url" -o "$dest"
  else
    wget -q --show-progress "$url" -O "$dest"
  fi
}

fetch_json() {
  url="$1"
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL -H "Accept: application/vnd.github.v3+json" "$url"
  else
    wget -qO- --header="Accept: application/vnd.github.v3+json" "$url"
  fi
}

# 3. Determine Latest Release & Assets
print_step "Checking for latest release from GitHub..."

RELEASE_JSON=$(fetch_json "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null || true)
TAG_NAME=$(printf "%s" "$RELEASE_JSON" | grep '"tag_name":' | head -n1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')

if [ -z "$TAG_NAME" ]; then
  TAG_NAME="v0.2.0"
  print_warning "GitHub API rate limit exceeded or offline; falling back to target release: $TAG_NAME"
fi

print_success "Target release: $TAG_NAME"

# Determine package type: check if dpkg is available and running on a Debian-derivative
PREFER_DEB=false
if command -v dpkg >/dev/null 2>&1 && [ -f /etc/debian_version ]; then
  PREFER_DEB=true
fi

# Allow override via environment or args
for arg in "$@"; do
  case "$arg" in
    --deb) PREFER_DEB=true ;;
    --appimage) PREFER_DEB=false ;;
  esac
done

TMP_DIR=$(mktemp -d -t cdus-install-XXXXXX)
trap 'rm -rf "$TMP_DIR"' EXIT

if [ "$PREFER_DEB" = true ]; then
  print_step "Locating Debian package (.deb)..."
  DEB_URL=$(printf "%s" "$RELEASE_JSON" | grep -Eo '"browser_download_url": *"[^"]+\.deb"' | head -n1 | sed -E 's/.*"browser_download_url": *"([^"]+)".*/\1/')
  
  if [ -z "$DEB_URL" ]; then
    DEB_URL="https://github.com/$REPO/releases/download/$TAG_NAME/cdus-desktop_${TAG_NAME#v}_amd64.deb"
  fi

  DEB_FILE="$TMP_DIR/cdus.deb"
  print_step "Downloading $DEB_URL..."
  if ! download_file "$DEB_URL" "$DEB_FILE"; then
    print_warning "Debian package download failed, attempting AppImage fallback..."
    PREFER_DEB=false
  else
    print_step "Installing CDUS via dpkg..."
    if [ "$(id -u)" -eq 0 ]; then
      dpkg -i "$DEB_FILE" || apt-get install -f -y
    else
      if command -v sudo >/dev/null 2>&1; then
        sudo dpkg -i "$DEB_FILE" || sudo apt-get install -f -y
      else
        print_error "'sudo' not found. Please install the downloaded package as root:"
        printf "  dpkg -i %s\n" "$DEB_FILE"
        exit 1
      fi
    fi
    print_success "CDUS Desktop installed successfully!"
  fi
fi

if [ "$PREFER_DEB" = false ]; then
  print_step "Locating standalone AppImage..."
  APPIMAGE_URL=$(printf "%s" "$RELEASE_JSON" | grep -Eo '"browser_download_url": *"[^"]+\.AppImage"' | head -n1 | sed -E 's/.*"browser_download_url": *"([^"]+)".*/\1/')

  if [ -z "$APPIMAGE_URL" ]; then
    APPIMAGE_URL="https://github.com/$REPO/releases/download/$TAG_NAME/cdus-desktop_${TAG_NAME#v}_amd64.AppImage"
  fi

  BIN_DIR="$HOME/.local/bin"
  mkdir -p "$BIN_DIR"
  TARGET_PATH="$BIN_DIR/cdus"

  print_step "Downloading AppImage to $TARGET_PATH..."
  download_file "$APPIMAGE_URL" "$TARGET_PATH"
  chmod +x "$TARGET_PATH"

  print_success "CDUS binary installed to $TARGET_PATH"

  # Check if $BIN_DIR is in PATH
  case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *)
      print_warning "Notice: $BIN_DIR is not in your current PATH."
      printf "Add it to your shell config (~/.bashrc or ~/.zshrc):\n"
      printf "  export PATH=\"\$HOME/.local/bin:\$PATH\"\n"
      ;;
  esac
fi

printf "\n${COLOR_GREEN}${COLOR_BOLD}Installation complete!${COLOR_RESET}\n"
printf "Run CDUS from your application menu or launch via terminal:\n"
printf "  cdus\n\n"
