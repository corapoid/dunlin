#!/usr/bin/env bash
# Build a standalone relay binary for the current machine.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

cargo build --release -p dunlin-relay
mkdir -p dist
cp target/release/dunlin-relay dist/dunlin-relay
chmod +x dist/dunlin-relay

echo "Built dist/dunlin-relay"
echo "Run: ./dist/dunlin-relay serve"
