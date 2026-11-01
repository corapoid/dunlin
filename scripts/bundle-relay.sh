#!/usr/bin/env bash
# Build a standalone relay binary for the current machine.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo build --release -p dunlin-relay --locked
DIST="${DUNLIN_DIST_DIR:-dist}"
mkdir -p "$DIST"
cp target/release/dunlin-relay "$DIST/dunlin-relay"
chmod +x "$DIST/dunlin-relay"

echo "Built $DIST/dunlin-relay"
echo "Run: $DIST/dunlin-relay serve"
