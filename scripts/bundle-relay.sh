#!/usr/bin/env bash
# Build a standalone relay binary for the current machine.
# Use --no-build to package the relay already built by bundle-macos.sh --build.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

if [[ "$#" -gt 1 ]]; then
  echo "Usage: $0 [--no-build]" >&2
  exit 1
fi
case "${1:-}" in
  "") cargo build --release -p dunlin-relay --locked ;;
  --no-build) ;;
  *) echo "Usage: $0 [--no-build]" >&2; exit 1 ;;
esac
if [[ ! -x target/release/dunlin-relay ]]; then
  echo "Missing release relay binary. Run a release build before --no-build." >&2
  exit 1
fi
DIST="${DUNLIN_DIST_DIR:-dist}"
mkdir -p "$DIST"
cp target/release/dunlin-relay "$DIST/dunlin-relay"
chmod +x "$DIST/dunlin-relay"

echo "Built $DIST/dunlin-relay"
echo "Run: $DIST/dunlin-relay serve"
