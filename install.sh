#!/usr/bin/env bash
# docker-diet installer for Linux / macOS / WSL2
# Usage: bash <(curl -sSf https://jawadboulmal.com/envault/install.sh)

set -e

# ── Config ────────────────────────────────────────────────────────────────────
REPO="Skayologie/docker-diet"
BIN="docker-diet"
INSTALL_DIR="$HOME/.local/bin"
# ─────────────────────────────────────────────────────────────────────────────

BOLD="\033[1m"; CYAN="\033[36m"; GREEN="\033[32m"; YELLOW="\033[33m"; RED="\033[31m"; RESET="\033[0m"

step() { echo -e "\n${CYAN}  >> $1${RESET}"; }
ok()   { echo -e "     ${GREEN}OK${RESET}  $1"; }
warn() { echo -e "     ${YELLOW}!!${RESET}  $1"; }
err()  { echo -e "\n${RED}  ERR: $1${RESET}\n"; exit 1; }

echo ""
echo -e "${BOLD}${CYAN}  docker-diet  —  Installer${RESET}"
echo -e "${CYAN}  ──────────────────────────────────────${RESET}"
echo ""

# ── 1. Detect platform ────────────────────────────────────────────────────────

step "Detecting platform..."

OS="$(uname -s)"
ARCH="$(uname -m)"

case "$OS" in
  Linux)
    case "$ARCH" in
      x86_64)  ASSET="${BIN}-linux-x86_64" ;;
      aarch64) ASSET="${BIN}-linux-aarch64" ;;
      *)        err "Unsupported architecture: $ARCH" ;;
    esac
    ;;
  Darwin)
    case "$ARCH" in
      arm64|aarch64) ASSET="${BIN}-macos-aarch64" ;;
      x86_64)        ASSET="${BIN}-macos-x86_64" ;;
      *)              err "Unsupported architecture: $ARCH" ;;
    esac
    ;;
  *)
    err "Unsupported OS: $OS. Use install.ps1 on Windows."
    ;;
esac

ok "Platform: $OS $ARCH  →  $ASSET"

# ── 2. Fetch latest release from GitHub ──────────────────────────────────────

step "Fetching latest release..."

API_URL="https://api.github.com/repos/$REPO/releases/latest"

if command -v curl &>/dev/null; then
  RELEASE_JSON=$(curl -sSfL -H "User-Agent: docker-diet-installer" "$API_URL")
elif command -v wget &>/dev/null; then
  RELEASE_JSON=$(wget -qO- --header="User-Agent: docker-diet-installer" "$API_URL")
else
  err "curl or wget is required but neither was found."
fi

VERSION=$(echo "$RELEASE_JSON" | grep '"tag_name"' | head -1 | sed 's/.*"tag_name": *"\([^"]*\)".*/\1/')
DOWNLOAD_URL=$(echo "$RELEASE_JSON" | grep '"browser_download_url"' | grep "$ASSET" | head -1 | sed 's/.*"browser_download_url": *"\([^"]*\)".*/\1/')

[ -z "$VERSION" ]      && err "Could not determine latest release version."
[ -z "$DOWNLOAD_URL" ] && err "No binary found for '$ASSET' in release $VERSION."

ok "Latest version : $VERSION"
ok "Download target: $ASSET"

# ── 3. Download ───────────────────────────────────────────────────────────────

step "Downloading $BIN $VERSION..."

mkdir -p "$INSTALL_DIR"
TMP="$(mktemp)"

if command -v curl &>/dev/null; then
  curl -sSfL "$DOWNLOAD_URL" -o "$TMP"
else
  wget -qO "$TMP" "$DOWNLOAD_URL"
fi

chmod +x "$TMP"
mv "$TMP" "$INSTALL_DIR/$BIN"

ok "Installed to: $INSTALL_DIR/$BIN"

# ── 4. Add to PATH ────────────────────────────────────────────────────────────

step "Checking PATH..."

if echo "$PATH" | grep -q "$INSTALL_DIR"; then
  ok "Already in PATH."
else
  SHELL_RC=""
  if [ -n "$ZSH_VERSION" ] || [ "$(basename "$SHELL")" = "zsh" ]; then
    SHELL_RC="$HOME/.zshrc"
  else
    SHELL_RC="$HOME/.bashrc"
  fi

  echo "" >> "$SHELL_RC"
  echo "export PATH=\"$INSTALL_DIR:\$PATH\"" >> "$SHELL_RC"
  export PATH="$INSTALL_DIR:$PATH"

  ok "Added $INSTALL_DIR to PATH in $SHELL_RC"
fi

# ── 5. Verify ─────────────────────────────────────────────────────────────────

step "Verifying..."

if "$INSTALL_DIR/$BIN" --version &>/dev/null; then
  ok "$(\"$INSTALL_DIR/$BIN\" --version) is ready."
else
  warn "Installed but could not verify. Open a new terminal and run: docker-diet --help"
fi

# ── Done ──────────────────────────────────────────────────────────────────────

echo ""
echo -e "${GREEN}${BOLD}  ✓ Installation complete!${RESET}"
echo ""
echo -e "  Quick start:"
echo "    docker-diet --help"
echo "    docker-diet dry-run  --image nginx:latest"
echo "    docker-diet analyze  --image myapp:latest"
echo ""
echo -e "${YELLOW}  NOTE: Open a new terminal if the command is not found yet.${RESET}"
echo ""
