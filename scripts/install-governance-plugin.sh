#!/bin/sh
# Install the built governance WASM pack into Dunlin's external plugin folder.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

if [ ! -f dist/governance/plugin.json ] || [ ! -f dist/governance/plugin.wasm ]; then
    ./scripts/build-governance-plugin.sh
fi

if [ -n "${DUNLIN_DATA_DIR:-}" ]; then
    DATA="$DUNLIN_DATA_DIR"
elif [ "$(uname -s)" = "Darwin" ]; then
    DATA="$HOME/Library/Application Support/Dunlin"
else
    DATA="${XDG_DATA_HOME:-$HOME/.local/share}/dunlin"
fi

TARGET="$DATA/plugins/dunlin-governance"
mkdir -p "$TARGET"
cp dist/governance/plugin.json "$TARGET/plugin.json"
cp dist/governance/plugin.wasm "$TARGET/plugin.wasm"
echo "Installed governance pack in $TARGET"
echo "It is disabled by default. Enable it in Settings → Extensions."
