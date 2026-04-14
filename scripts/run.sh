#!/usr/bin/env bash
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(cd "$SCRIPT_DIR/.." && pwd)"

PROFILE="${1:-debug}"
APP="$PROJECT_DIR/target/TAI.app"

case "$PROFILE" in
    release|debug) ;;
    app)
        if [ ! -d "$APP" ]; then
            echo "TAI.app not found — run: scripts/build.sh release && scripts/bundle.sh" >&2
            exit 1
        fi
        open "$APP"
        exit 0
        ;;
    *)
        echo "Usage: $0 [debug|release|app] [args...]" >&2
        exit 1
        ;;
esac

cd "$PROJECT_DIR"
cargo run $([ "$PROFILE" = "release" ] && echo "--release") -- "${@:2}"
