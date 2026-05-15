#!/bin/bash
# Build script for AppImage that works around VS Code snap pollution
# Requires: librsvg2-dev (sudo apt install librsvg2-dev)
#
# The issue: VS Code snap sets GDK_PIXBUF_MODULE_FILE to its own loader cache,
# which causes linuxdeploy's gtk plugin to load snap libraries that are
# incompatible with the system glibc.
#
# This script unsets the problematic environment variables before building.

set -e

# Clear snap-polluted environment
unset GDK_PIXBUF_MODULE_FILE
export GDK_PIXBUF_MODULE_FILE=/usr/lib/x86_64-linux-gnu/gdk-pixbuf-2.0/2.10.0/loaders.cache

# Verify required dev package
if ! pkg-config --exists librsvg-2.0 2>/dev/null; then
    echo "ERROR: librsvg2-dev is not installed."
    echo "Run: sudo apt install librsvg2-dev"
    exit 1
fi

cd "$(dirname "$0")"
cargo tauri build --bundles appimage

echo "AppImage built at: target/release/bundle/appimage/"
