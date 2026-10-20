#!/bin/sh
# Installs (or removes) kiln from the latest GitHub release.
#
#   curl -fsSL https://raw.githubusercontent.com/ReyArlena/kiln/main/install.sh | sudo sh   # system-wide
#   curl -fsSL https://raw.githubusercontent.com/ReyArlena/kiln/main/install.sh | sh        # this user only
#   sh install.sh --uninstall     # remove the binary
#   sh install.sh --purge         # remove the binary, settings and run history
#
# Run as root it installs to /usr/local/bin, so plain `sudo kiln` works (most
# distros give sudo a fixed secure_path that leaves out ~/.local/bin). Run as
# a normal user it installs to ~/.local/bin and kiln must usually be given by
# full path under sudo. Override the location with KILN_BIN_DIR.
# To build from source instead, see the Develop section of the README.
set -e

REPO="ReyArlena/kiln"
if [ -n "$KILN_BIN_DIR" ]; then
    BIN_DIR="$KILN_BIN_DIR"
elif [ "$(id -u)" = "0" ]; then
    BIN_DIR="/usr/local/bin"
else
    BIN_DIR="$HOME/.local/bin"
fi

# Settings live in the invoking user's home even when run through sudo.
if [ -n "$SUDO_USER" ] && [ "$(id -u)" = "0" ]; then
    USER_HOME=$(getent passwd "$SUDO_USER" | cut -d: -f6)
else
    USER_HOME="$HOME"
fi
CONFIG_DIR="${XDG_CONFIG_HOME:-$USER_HOME/.config}/kiln"
DATA_DIR="${XDG_DATA_HOME:-$USER_HOME/.local/share}/kiln"

case "$1" in
    --uninstall|--purge)
        removed=0
        needs_root=0
        for bin in /usr/local/bin/kiln "$USER_HOME/.local/bin/kiln" "$HOME/.local/bin/kiln" "$BIN_DIR/kiln"; do
            [ -f "$bin" ] || continue
            if [ -w "$(dirname "$bin")" ]; then
                rm -f "$bin"
                echo "Removed $bin"
                removed=1
            else
                needs_root=1
                system_bin="$bin"
            fi
        done
        if [ "$needs_root" = "1" ]; then
            echo "kiln is installed system-wide at $system_bin, which needs root to remove." >&2
            echo "Rerun with sudo:" >&2
            if [ -f "$0" ] && [ "$0" != "sh" ]; then
                echo "  sudo sh $0 $1" >&2
            else
                echo "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | sudo sh -s -- $1" >&2
            fi
            exit 1
        fi
        [ "$removed" = "1" ] || echo "kiln was not installed"
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
if [ "$BIN_DIR" = "/usr/local/bin" ]; then
    echo "Most tests need root:"
    echo "  sudo kiln"
else
    echo "Most tests need root. sudo usually drops ~/.local/bin from PATH, so run it by full path:"
    echo "  sudo $BIN_DIR/kiln"
    echo "Or install system-wide so plain 'sudo kiln' works:"
    echo "  curl -fsSL https://raw.githubusercontent.com/$REPO/main/install.sh | sudo sh"
fi
echo
echo "Prefer to compile it yourself? See the Develop section of the README."
