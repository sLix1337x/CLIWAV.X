#!/usr/bin/env bash
# CLIWAV.X installer for macOS. Counterpart to install.ps1.
#
# Fetches the release binary matching this Mac's architecture, installs it to
# ~/.local/bin, and pulls in mpv/yt-dlp through Homebrew if they're missing.
#
#   curl -fsSL https://sLix1337x.github.io/CLIWAV.X/install.sh | bash
set -euo pipefail

REPO="sLix1337x/CLIWAV.X"
INSTALL_DIR="${HOME}/.local/bin"

# Colors only when stdout is a terminal — piping this into a file or a log
# shouldn't litter it with escape sequences.
if [ -t 1 ]; then
    CYAN=$'\033[36m'; GREEN=$'\033[32m'; YELLOW=$'\033[33m'; RED=$'\033[31m'; RESET=$'\033[0m'
else
    CYAN=''; GREEN=''; YELLOW=''; RED=''; RESET=''
fi

info()  { printf '%s%s%s\n' "$CYAN"   "$1" "$RESET"; }
ok()    { printf '%s%s%s\n' "$GREEN"  "$1" "$RESET"; }
warn()  { printf '%s%s%s\n' "$YELLOW" "$1" "$RESET" >&2; }
fail()  { printf '%s%s%s\n' "$RED"    "$1" "$RESET" >&2; exit 1; }

has() { command -v "$1" >/dev/null 2>&1; }

[ "$(uname -s)" = "Darwin" ] || fail "This installer is for macOS. On Windows use install.ps1."

case "$(uname -m)" in
    arm64)  ASSET="cliwavx-macos-arm64" ;;
    x86_64) ASSET="cliwavx-macos-x86_64" ;;
    *)      fail "Unsupported architecture: $(uname -m)" ;;
esac

info "Installing CLIWAV.X and its dependencies..."

# --- mpv and yt-dlp ---
# Only the genuinely missing ones get installed, so re-running to update
# CLIWAV.X doesn't churn through Homebrew every time.
missing=()
has mpv    || missing+=("mpv")
has yt-dlp || missing+=("yt-dlp")

if [ ${#missing[@]} -gt 0 ]; then
    if has brew; then
        info "Installing via Homebrew: ${missing[*]}"
        brew install "${missing[@]}"
    else
        warn "Homebrew not found, so ${missing[*]} could not be installed automatically."
        warn "Install Homebrew from https://brew.sh and re-run this script, or install"
        warn "${missing[*]} manually and make sure they are on your PATH."
    fi
else
    ok "mpv and yt-dlp are already installed."
fi

# --- CLIWAV.X binary ---
URL="https://github.com/${REPO}/releases/download/latest/${ASSET}"
TMP="$(mktemp -d)"
# Clean up the temp dir even if the download or install fails partway.
trap 'rm -rf "$TMP"' EXIT

info "Downloading ${ASSET}..."
if ! curl -fsSL "$URL" -o "${TMP}/cliwavx"; then
    warn "Could not download ${ASSET} from:"
    warn "  $URL"
    fail "Check your connection, or see https://github.com/${REPO}/releases/latest"
fi

mkdir -p "$INSTALL_DIR"
# Install to a temp name and move into place: replacing a running binary
# in-place fails with "Text file busy", while a rename is atomic.
chmod +x "${TMP}/cliwavx"
mv -f "${TMP}/cliwavx" "${INSTALL_DIR}/cliwavx"

# "wavx" is a symlink rather than a copy — one binary on disk, and it keeps
# pointing at the right file when cliwavx is replaced on the next update.
ln -sf "${INSTALL_DIR}/cliwavx" "${INSTALL_DIR}/wavx"

# macOS quarantines anything downloaded with a browser-style user agent; curl
# usually avoids that, but strip the attribute if it did get set, otherwise
# Gatekeeper blocks the first launch.
xattr -d com.apple.quarantine "${INSTALL_DIR}/cliwavx" 2>/dev/null || true

ok "CLIWAV.X installed to ${INSTALL_DIR}"

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*)
        ok "Run 'cliwavx' (or the shorter 'wavx') from anywhere."
        ;;
    *)
        warn ""
        warn "${INSTALL_DIR} is not on your PATH. Add it with:"
        warn "  echo 'export PATH=\"\$HOME/.local/bin:\$PATH\"' >> ~/.zshrc"
        warn "Then restart your terminal and run 'cliwavx'."
        ;;
esac
