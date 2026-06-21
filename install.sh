#!/usr/bin/env bash
# Docker-Diet Installer for Linux / macOS / WSL2
# Run with: curl -sSf https://raw.githubusercontent.com/.../install.sh | bash
# Or locally: bash install.sh

set -e

BOLD="\033[1m"; CYAN="\033[36m"; GREEN="\033[32m"; YELLOW="\033[33m"; RESET="\033[0m"

step() { echo -e "\n${CYAN}>> $1${RESET}"; }
ok()   { echo -e "   ${GREEN}OK${RESET}  $1"; }
warn() { echo -e "   ${YELLOW}!!${RESET}  $1"; }

echo ""
echo -e "${BOLD}${CYAN}  docker-diet installer${RESET}"
echo -e "${BOLD}  ─────────────────────────────────────${RESET}"
echo ""

# ── 1. Check / install Rust ───────────────────────────────────────────────────

step "Checking for Rust / cargo..."

if ! command -v cargo &>/dev/null; then
    warn "Rust not found. Installing via rustup..."
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable
    source "$HOME/.cargo/env"
    ok "Rust installed."
else
    ok "Rust already installed: $(cargo --version)"
fi

# Ensure cargo env is loaded
[ -f "$HOME/.cargo/env" ] && source "$HOME/.cargo/env"

# ── 2. Build and install docker-diet ─────────────────────────────────────────

step "Building and installing docker-diet (first build: 2-5 minutes)..."

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$SCRIPT_DIR"

cargo install --path diet-cli --quiet
ok "docker-diet installed."

# ── 3. Verify ─────────────────────────────────────────────────────────────────

step "Verifying installation..."

if command -v docker-diet &>/dev/null; then
    ok "Binary : $(which docker-diet)"
    ok "Version: $(docker-diet --version)"
else
    warn "Binary not found in PATH. Add ~/.cargo/bin to your PATH:"
    echo '    echo '"'"'export PATH="$HOME/.cargo/bin:$PATH"'"'"' >> ~/.bashrc && source ~/.bashrc'
fi

# ── 4. Done ───────────────────────────────────────────────────────────────────

echo ""
echo -e "${GREEN}${BOLD}  Installation complete!${RESET}"
echo ""
echo -e "  Usage examples:"
echo "    docker-diet dry-run  --image nginx:latest"
echo "    docker-diet analyze  --image myapp:latest"
echo "    docker-diet analyze  --tarball ./app.tar --output ./slim.tar"
echo "    docker-diet --help"
echo ""
