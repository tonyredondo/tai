#!/usr/bin/env bash
set -euo pipefail

PROFILE="${1:-debug}"

case "$PROFILE" in
    release)
        cargo run --release -- "${@:2}"
        ;;
    debug)
        cargo run -- "${@:2}"
        ;;
    *)
        echo "Usage: $0 [debug|release] [args...]" >&2
        exit 1
        ;;
esac
