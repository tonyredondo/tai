#!/usr/bin/env bash
set -euo pipefail

PROFILE="${1:-release}"

case "$PROFILE" in
    release)
        cargo build --release
        echo "Build complete: target/release/tai"
        ;;
    debug)
        cargo build
        echo "Build complete: target/debug/tai"
        ;;
    *)
        echo "Usage: $0 [release|debug]" >&2
        exit 1
        ;;
esac
