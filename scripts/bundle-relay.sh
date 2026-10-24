#!/usr/bin/env bash
# Build a standalone relay binary for the current machine.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo build --release -p kestrel-relay
mkdir -p dist
cp target/release/kestrel-relay dist/kestrel-relay
chmod +x dist/kestrel-relay

echo "Built dist/kestrel-relay"
echo "Run: ./dist/kestrel-relay serve"
