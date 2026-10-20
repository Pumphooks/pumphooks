#!/usr/bin/env bash
# Build the core program and the test/reference hooks into target/deploy.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
for m in programs/hooks-core "$@"; do
  cargo build-sbf --manifest-path "$ROOT/$m/Cargo.toml" --sbf-out-dir "$ROOT/target/deploy"
done
ls -la "$ROOT"/target/deploy/*.so
