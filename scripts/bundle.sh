#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

PROFILE="${1:-release}"
case "$PROFILE" in
    release) BIN_DIR="$PROJECT_DIR/target/release" ;;
    debug)   BIN_DIR="$PROJECT_DIR/target/debug" ;;
    *)       echo "Usage: $0 [release|debug]" >&2; exit 1 ;;
esac

BINARY="$BIN_DIR/tai"
if [ ! -f "$BINARY" ]; then
    echo "Binary not found at $BINARY — run scripts/build.sh $PROFILE first" >&2
    exit 1
fi

APP="$PROJECT_DIR/target/TAI.app"
rm -rf "$APP"

mkdir -p "$APP/Contents/MacOS"
mkdir -p "$APP/Contents/Resources"

cp "$PROJECT_DIR/assets/Info.plist" "$APP/Contents/"
cp "$BINARY"                        "$APP/Contents/MacOS/tai"
cp "$PROJECT_DIR/assets/tai.icns"   "$APP/Contents/Resources/tai.icns"
cp "$PROJECT_DIR/assets/icon.png"   "$APP/Contents/Resources/icon.png"

# Copy fonts if present
if [ -d "$PROJECT_DIR/fonts" ]; then
    cp -r "$PROJECT_DIR/fonts" "$APP/Contents/Resources/fonts"
fi

echo "Created $APP"
