#!/bin/sh
# Build the disabled-by-default governance WASM plugin into dist/governance/.
set -eu

ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
cd "$ROOT"

if ! command -v cargo >/dev/null 2>&1; then
    if [ -x "$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin/cargo" ]; then
        PATH="$HOME/.rustup/toolchains/stable-aarch64-apple-darwin/bin:$PATH"
        export PATH
    else
        echo "cargo is required" >&2
        exit 1
    fi
fi

rustup target add wasm32-unknown-unknown
cargo build --manifest-path plugins/governance/Cargo.toml \
    --target wasm32-unknown-unknown --release

WASM="plugins/governance/target/wasm32-unknown-unknown/release/dunlin_governance_plugin.wasm"
if [ ! -f "$WASM" ]; then
    echo "WASM build did not produce $WASM" >&2
    exit 1
fi

OUT="dist/governance"
mkdir -p "$OUT"
cp "$WASM" "$OUT/plugin.wasm"
if command -v shasum >/dev/null 2>&1; then
    HASH=$(shasum -a 256 "$OUT/plugin.wasm" | awk '{print $1}')
elif command -v sha256sum >/dev/null 2>&1; then
    HASH=$(sha256sum "$OUT/plugin.wasm" | awk '{print $1}')
else
    echo "A SHA-256 tool (shasum or sha256sum) is required" >&2
    exit 1
fi
sed "s/__SHA256__/$HASH/" plugins/governance/plugin.template.json > "$OUT/plugin.json"
echo "Built $OUT/plugin.json"
echo "sha256 $HASH"
