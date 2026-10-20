#!/usr/bin/env bash
# Build the on-chain program into target/deploy (the path the integration tests load), regardless
# of CARGO_TARGET_DIR. rustup's cargo must precede any Homebrew cargo for `+toolchain` to work.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
cargo build-sbf --manifest-path "$ROOT/programs/pump-hooks/Cargo.toml" --sbf-out-dir "$ROOT/target/deploy" "$@"
ls -la "$ROOT/target/deploy/pump_hooks.so"
