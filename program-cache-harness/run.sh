#!/usr/bin/env bash

set -euo pipefail

here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
manifest="${here}/../dev-bins/Cargo.toml"
mode="${1:-once}"
shift || true

case "$mode" in
    once)
        exec cargo run --manifest-path "$manifest" -p agave-program-cache-harness \
            --bin run_once "$@"
        ;;
    shuttle)
        exec cargo test --manifest-path "$manifest" -p agave-program-cache-harness \
            --features shuttle-test --test shuttle "$@"
        ;;
    *)
        echo "unknown mode: $mode (expected 'once' or 'shuttle')" >&2
        exit 1
        ;;
esac
