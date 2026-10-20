#!/bin/sh
# Installs (or removes) kiln for the current user.
#
#   curl -fsSL https://raw.githubusercontent.com/ReyArlena/kiln/main/install.sh | sh
#   sh install.sh                 # from a local clone (builds from source)
#   sh install.sh --uninstall     # remove the binary
#   sh install.sh --purge         # remove the binary, settings and run history
#
# Installs to ~/.local/bin. Builds with cargo when available; otherwise
# downloads a prebuilt binary from the latest GitHub release.
set -e

REPO="ReyArlena/kiln"
BIN_DIR="${KILN_BIN_DIR:-$HOME/.local/bin}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/kiln"
DATA_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/kiln"
ASSET="kiln-x86_64-unknown-linux-musl"

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
esac

mkdir -p "$BIN_DIR"
src_dir=$(cd "$(dirname "$0")" 2>/dev/null && pwd || echo "")

if command -v cargo >/dev/null 2>&1; then
    if [ -n "$src_dir" ] && [ -f "$src_dir/Cargo.toml" ]; then
        echo "Building from $src_dir"
        cargo install --path "$src_dir" --root "${BIN_DIR%/bin}" --locked
    else
        echo "Building from https://github.com/$REPO"
        cargo install --git "https://github.com/$REPO" --root "${BIN_DIR%/bin}" --locked
    fi
else
    if ! command -v curl >/dev/null 2>&1; then
        echo "Need either cargo (https://rustup.rs) or curl" >&2
        exit 1
    fi
    url="https://github.com/$REPO/releases/latest/download/$ASSET"
    echo "No cargo found, downloading $url"
    tmp=$(mktemp "$BIN_DIR/.kiln.XXXXXX")
    trap 'rm -f "$tmp"' EXIT
    if ! curl -fsSL "$url" -o "$tmp"; then
        echo "Download failed. Install the Rust toolchain (https://rustup.rs) and rerun, or build from a clone." >&2
        exit 1
    fi
    chmod 755 "$tmp"
    mv -f "$tmp" "$BIN_DIR/kiln"
    trap - EXIT
fi

echo
echo "Installed $BIN_DIR/kiln"
case ":$PATH:" in
    *":$BIN_DIR:"*) ;;
    *) echo "Note: $BIN_DIR is not on your PATH" ;;
esac
echo
echo "Most tests need root. sudo resets PATH, so run it by full path:"
echo "  sudo $BIN_DIR/kiln"
