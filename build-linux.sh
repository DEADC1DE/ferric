#!/usr/bin/env bash
#
# build-linux.sh
# Builds the Ferric application on Linux.
#
# Usage:
#   ./build-linux.sh              # Release build (default)
#   ./build-linux.sh --debug      # Debug build
#   ./build-linux.sh --test       # Run workspace tests first
#   ./build-linux.sh --clean      # Clean previous build artifacts
#   ./build-linux.sh --reinstall  # Force reinstall node_modules (npm ci)
#   ./build-linux.sh --run        # Launch Ferric after a successful build
#   ./build-linux.sh --install    # Install binary and desktop launcher (KDE/GNOME)
#   ./build-linux.sh --no-deps    # Skip automatic dependency installation
#   ./build-linux.sh --help       # Show this help
#
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
UI_DIR="${ROOT_DIR}/ui"

MODE="release"
CARGO_FLAGS=(--release)
RUN_TESTS=false
CLEAN_FIRST=false
FORCE_REINSTALL=false
RUN_AFTER=false
INSTALL_DEPS=true
INSTALL_DESKTOP=false

for arg in "$@"; do
  case "$arg" in
    --debug)
      MODE="debug"
      CARGO_FLAGS=()
      ;;
    --test)
      RUN_TESTS=true
      ;;
    --clean)
      CLEAN_FIRST=true
      ;;
    --reinstall)
      FORCE_REINSTALL=true
      ;;
    --run)
      RUN_AFTER=true
      ;;
    --install|--desktop)
      INSTALL_DESKTOP=true
      ;;
    --no-deps)
      INSTALL_DEPS=false
      ;;
    --help|-h)
      sed -n '3,16p' "$0" | sed 's/^# \?//'
      exit 0
      ;;
    *)
      echo "Unknown argument: $arg" >&2
      echo "Use './build-linux.sh --help' for usage." >&2
      exit 1
      ;;
  esac
done

step() {
  echo ""
  echo -e "\033[1;36m==> $1\033[0m"
}

warn() {
  echo -e "\033[1;33m[Warning]\033[0m $1"
}

require_cmd() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "Error: '$1' is not installed or not in PATH." >&2
    exit 1
  fi
}

SUDO=""
if [ "$(id -u)" -ne 0 ]; then
  if command -v sudo >/dev/null 2>&1; then
    SUDO="sudo"
  fi
fi

if [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
elif [ -d "$HOME/.cargo/bin" ]; then
  export PATH="$HOME/.cargo/bin:$PATH"
fi

if [ "$INSTALL_DEPS" = true ]; then
  # 1. System packages check
  NEED_SYS_PKGS=false
  if ! command -v pkg-config >/dev/null 2>&1 || ! command -v curl >/dev/null 2>&1 || ! command -v gcc >/dev/null 2>&1; then
    NEED_SYS_PKGS=true
  else
    for pkg in webkit2gtk-4.1 ayatana-appindicator3-0.1 librsvg-2.0 openssl; do
      if ! pkg-config --exists "$pkg" 2>/dev/null; then
        NEED_SYS_PKGS=true
        break
      fi
    done
  fi

  if [ "$NEED_SYS_PKGS" = true ]; then
    step "Installing required system packages..."
    if command -v apt-get >/dev/null 2>&1; then
      $SUDO apt-get update
      $SUDO apt-get install -y libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev libssl-dev pkg-config build-essential curl
    elif command -v dnf >/dev/null 2>&1; then
      $SUDO dnf install -y webkit2gtk4.1-devel libappindicator-gtk3-devel librsvg2-devel openssl-devel pkgconf-pkg-config gcc gcc-c++ make curl
    elif command -v pacman >/dev/null 2>&1; then
      $SUDO pacman -S --needed --noconfirm webkit2gtk-4.1 libayatana-appindicator librsvg openssl pkgconf base-devel curl
    elif command -v zypper >/dev/null 2>&1; then
      $SUDO zypper install -y webkit2gtk-4.1-devel libayatana-appindicator3-devel librsvg-devel libopenssl-devel pkg-config patterns-devel-base-devel_basis curl
    else
      warn "Package manager not recognized. Please install system dependencies manually."
    fi
  fi

  # 2. Node & npm check
  if ! command -v node >/dev/null 2>&1 || ! command -v npm >/dev/null 2>&1; then
    step "Installing Node.js and npm..."
    if command -v apt-get >/dev/null 2>&1; then
      $SUDO apt-get install -y nodejs npm
    elif command -v dnf >/dev/null 2>&1; then
      $SUDO dnf install -y nodejs npm
    elif command -v pacman >/dev/null 2>&1; then
      $SUDO pacman -S --needed --noconfirm nodejs npm
    fi
  fi

  # 3. Rust & Cargo check
  if ! command -v cargo >/dev/null 2>&1; then
    step "Installing Rust toolchain (rustup)..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --default-toolchain stable
    if [ -f "$HOME/.cargo/env" ]; then
      . "$HOME/.cargo/env"
    elif [ -d "$HOME/.cargo/bin" ]; then
      export PATH="$HOME/.cargo/bin:$PATH"
    fi
  fi
fi

require_cmd node
require_cmd npm
require_cmd cargo

echo "node:  $(node --version)"
echo "npm:   $(npm --version)"
echo "cargo: $(cargo --version)"

# Clean if requested
if [ "$CLEAN_FIRST" = true ]; then
  step "Cleaning previous builds"
  rm -rf "${UI_DIR}/dist"
  cargo clean -p ferric-app
fi

# Run tests if requested
if [ "$RUN_TESTS" = true ]; then
  step "Running workspace tests (cargo test --workspace)"
  (cd "$ROOT_DIR" && cargo test --workspace)
fi

# Build UI
step "Building frontend (Vite)"
(
  cd "$UI_DIR"
  if [ "$FORCE_REINSTALL" = true ] || [ ! -d "node_modules" ]; then
    if [ -f "package-lock.json" ]; then
      npm ci
    else
      npm install
    fi
  else
    echo "node_modules present, skipping install."
  fi
  npm run build
)

if [ ! -f "${UI_DIR}/dist/index.html" ]; then
  echo "Error: ui/dist/index.html was not created." >&2
  exit 1
fi

# Build Rust app
step "Building Ferric ($MODE) with Cargo"
(
  cd "$ROOT_DIR"
  cargo build "${CARGO_FLAGS[@]}" -p ferric-app
)

BIN_PATH="${ROOT_DIR}/target/${MODE}/ferric-app"
if [ ! -f "$BIN_PATH" ]; then
  echo "Error: $BIN_PATH was not created." >&2
  exit 1
fi

SIZE_MB=$(du -m "$BIN_PATH" | cut -f1)
echo ""
echo -e "\033[1;32mDone: ${BIN_PATH} (~${SIZE_MB} MB)\033[0m"

if [ "$INSTALL_DESKTOP" = true ]; then
  step "Installing Ferric desktop integration (KDE/GNOME)..."
  BIN_DEST="${HOME}/.local/bin"
  DESKTOP_DEST="${HOME}/.local/share/applications"
  ICON_DEST="${HOME}/.local/share/icons/hicolor"

  mkdir -p "$BIN_DEST" "$DESKTOP_DEST" "$ICON_DEST/128x128/apps" "$ICON_DEST/32x32/apps" "$ICON_DEST/scalable/apps"

  cp "$BIN_PATH" "$BIN_DEST/ferric-app"
  cp "${ROOT_DIR}/crates/ferric-app/icons/128x128.png" "$ICON_DEST/128x128/apps/ferric.png"
  cp "${ROOT_DIR}/crates/ferric-app/icons/32x32.png" "$ICON_DEST/32x32/apps/ferric.png"
  cp "${ROOT_DIR}/crates/ferric-app/icons/icon.png" "$ICON_DEST/scalable/apps/ferric.png"

  sed "s|Exec=ferric-app|Exec=${BIN_DEST}/ferric-app|" "${ROOT_DIR}/ferric.desktop" > "$DESKTOP_DEST/ferric.desktop"
  chmod +x "$DESKTOP_DEST/ferric.desktop"

  if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DESKTOP_DEST" 2>/dev/null || true
  fi

  echo -e "\033[1;32mInstalled to ${BIN_DEST}/ferric-app and registered for KDE/GNOME.\033[0m"
fi

if [ "$RUN_AFTER" = true ]; then
  step "Starting Ferric..."
  if [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
    warn "No DISPLAY or WAYLAND_DISPLAY detected in current session."
    echo "Ferric requires a graphical display (X11/Wayland)."
    echo ""
    echo "Start options:"
    echo "  1. Headless test:       xvfb-run ./build-linux.sh --run"
    echo "  2. Remote via SSH:      ssh -X <user>@<host> (requires local X-Server)"
    echo "  3. Secure Remote:       Xpra or VNC over SSH tunnel (recommended for isolation)"
    echo "  4. Local Desktop:       DISPLAY=:0 ./build-linux.sh --run"
    echo ""
    echo "Security notice on remote access:"
    echo "  - Plain X11 forwarding (ssh -X / -Y) allows app access to local keyboard & clipboard."
    echo "  - For sensitive environments, use an isolated tunnel (e.g. Xpra or TigerVNC)."
    echo ""
    exit 1
  fi
  if [ -n "${SSH_CONNECTION:-}" ] || [ -n "${SSH_CLIENT:-}" ]; then
    warn "SSH X11 forwarding detected (DISPLAY=${DISPLAY:-})."
    echo "  X11 forwarding allows app interaction with local keyboard and clipboard."
    echo "  For maximum isolation, an SSH VNC/Xpra tunnel is recommended."
  fi
  "$BIN_PATH"
fi
