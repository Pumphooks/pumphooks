#!/usr/bin/env bash
# Build every on-chain program into target/deploy (localnet / LiteSVM tests).
# Usage: scripts/build_programs.sh [program-dir ...]   (default: all)
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
export PATH="$HOME/.cargo/bin:$HOME/.local/share/solana/install/active_release/bin:$PATH"
PROGRAMS=("$@")
if [ ${#PROGRAMS[@]} -eq 0 ]; then
  PROGRAMS=(programs/*/)
fi
for m in "${PROGRAMS[@]}"; do
  cargo build-sbf --manifest-path "$ROOT/${m%/}/Cargo.toml" --sbf-out-dir "$ROOT/target/deploy"
done
ls -la "$ROOT"/target/deploy/*.so
