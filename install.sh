#!/bin/sh
# Installs (or removes) kiln for the current user from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/ReyArlena/kiln/main/install.sh | sh
#   sh install.sh --uninstall     # remove the binary
#   sh install.sh --purge         # remove the binary, settings and run history
#
# Installs a prebuilt static binary to ~/.local/bin (override with KILN_BIN_DIR).
# To build from source instead, see the Develop section of the README.
set -e

REPO="ReyArlena/kiln"
BIN_DIR="${KILN_BIN_DIR:-$HOME/.local/bin}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/kiln"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/kiln"

case "$1" in
    --uninstall|--purge)
        rm -f "$BIN_DIR/kiln"
        echo "Removed $BIN_DIR/kiln"
        if [ "$1" = "--purge" ]; then
            rm -rf "$CONFIG_DIR" "$DATA_DIR"
            echo "Removed $CONFIG_DIR and $DATA_DIR"
        else
            echo "Settings and run history kept in $CONFIG_DIR and $DATA_DIR (use --purge to remove)"
        fi
        exit 0
        ;;
    "") ;;
    *)
        echo "Usage: install.sh [--uninstall | --purge]" >&2
        exit 1
        ;;
esac

if ! command -v curl >/dev/null 2>&1; then
    echo "curl is required" >&2
    exit 1
fi

# Release assets are named kiln-linux-<arch>.
case "$(uname -s)-$(uname -m)" in
    Linux-x86_64|Linux-amd64) ASSET="kiln-linux-x86_64" ;;
    Linux-aarch64|Linux-arm64) ASSET="kiln-linux-aarch64" ;;
    *)
        echo "No prebuilt binary for $(uname -s) $(uname -m)." >&2
        echo "Build from source instead: see the Develop section of the README." >&2
        exit 1
        ;;
esac
url="https://github.com/$REPO/releases/latest/download/$ASSET"

mkdir -p "$BIN_DIR"
# Download to a temp file first, then move into place, so a failed download
# never leaves a truncated binary behind.
tmp=$(mktemp "$BIN_DIR/.kiln.XXXXXX")
trap 'rm -f "$tmp"' EXIT

echo "Downloading $url"
if ! curl -fsSL "$url" -o "$tmp"; then
    echo "Download failed. Check https://github.com/$REPO/releases, or build from source (see the README)." >&2
    exit 1
fi
chmod 755 "$tmp"
mv -f "$tmp" "$BIN_DIR/kiln"
trap - EXIT

echo
echo "Installed $BIN_DIR/kiln ($("$BIN_DIR/kiln" --version 2>/dev/null || echo 'version unknown'))"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH" ;;
esac
echo
echo "Most tests need root. sudo resets PATH, so run it by full path:"
echo "  sudo $BIN_DIR/kiln"
echo
echo "Prefer to compile it yourself? See the Develop section of the README."
