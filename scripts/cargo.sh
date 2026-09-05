#!/bin/bash

# Runs any cargo command inside the official Rust image, so the box needs no
# toolchain.
#
#   ./scripts/cargo.sh test
#   ./scripts/cargo.sh clippy --all-targets -- -D warnings
#   RUST_IMAGE=rust:1.85-slim ./scripts/cargo.sh check
#
# Both caches live in named docker VOLUMES rather than in the working tree, so
# neither target/ nor a registry checkout can end up in the repo or in a commit.
# They persist between runs, so only the first build pays for the index.

set -euo pipefail

cd "$(dirname "$0")/.."

RUST_IMAGE="${RUST_IMAGE:-rust:1-slim}"
TARGET_VOLUME="${TARGET_VOLUME:-internetdata-rust-target}"
CARGO_VOLUME="${CARGO_VOLUME:-internetdata-rust-cargo}"

# The four forwarded variables use docker's BARE -e form, which passes a
# variable through only when it is actually set: cargo refuses an EMPTY
# CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS outright, and an empty API key would
# be sent as a key rather than as none.
exec docker run --rm -i \
    -v "$PWD:/work" \
    -v "${TARGET_VOLUME}:/target" \
    -v "${CARGO_VOLUME}:/cargo" \
    -e CARGO_TARGET_DIR=/target \
    -e CARGO_HOME=/cargo \
    -e CARGO_TERM_COLOR=never \
    -e RUSTDOCFLAGS \
    -e CARGO_RESOLVER_INCOMPATIBLE_RUST_VERSIONS \
    -e INTERNETDATA_API_KEY \
    -e INTERNETDATA_BASE_URL \
    -w /work \
    "$RUST_IMAGE" bash -euc "
        # rust:*-slim ships neither component, and the one behind \`cargo fmt\` is
        # named rustfmt rather than fmt.
        case \"\${1:-}\" in
            clippy) rustup component add clippy >/dev/null ;;
            fmt) rustup component add rustfmt >/dev/null ;;
        esac
        exec cargo \"\$@\"
    " bash "$@"
