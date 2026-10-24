#!/usr/bin/env bash
# Package the Kestrel binary as a native macOS .app bundle.
#
# Usage:
#   ./scripts/bundle-macos.sh              # use target/release, then target/debug
#   ./scripts/bundle-macos.sh --build      # cargo build --release first
#   KESTREL_CODESIGN_IDENTITY="Developer ID Application: ..." \
#     ./scripts/bundle-macos.sh --build
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

BUILD=0
if [[ "${1:-}" == "--build" ]]; then
  BUILD=1
fi

if [[ "$BUILD" == "1" ]]; then
  cargo build --release -p kestrel
fi

BIN="${KESTREL_BIN:-}"
if [[ -z "$BIN" ]]; then
  if [[ -x "target/release/kestrel" ]]; then
    BIN="target/release/kestrel"
  elif [[ -x "target/debug/kestrel" ]]; then
    BIN="target/debug/kestrel"
  else
    echo "No Kestrel binary found. Run: cargo build -p kestrel" >&2
    exit 1
  fi
fi

APP="dist/Kestrel.app"
CONTENTS="$APP/Contents"
MACOS="$CONTENTS/MacOS"
RESOURCES="$CONTENTS/Resources"
ICONSET="dist/Kestrel.iconset"
MASTER="dist/kestrel-icon.png"

rm -rf "$APP" "$ICONSET"
mkdir -p "$MACOS" "$RESOURCES" "$ICONSET" "dist"

cp "$BIN" "$MACOS/Kestrel"
chmod +x "$MACOS/Kestrel"

# Render the bundled SVG icon and expand it into an .icns.
if command -v qlmanage >/dev/null 2>&1 && command -v iconutil >/dev/null 2>&1; then
  qlmanage -t -s 1024 -o dist assets/kestrel-icon.svg >/dev/null 2>&1
  if [[ -f "dist/kestrel-icon.svg.png" ]]; then
    mv "dist/kestrel-icon.svg.png" "$MASTER"
    for size in 16 32 128 256 512; do
      sips -z "$size" "$size" "$MASTER" --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
      double=$((size * 2))
      sips -z "$double" "$double" "$MASTER" --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
    done
    iconutil -c icns "$ICONSET" -o "$RESOURCES/Kestrel.icns"
  fi
fi

cat > "$CONTENTS/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key>
  <string>Kestrel</string>
  <key>CFBundleDisplayName</key>
  <string>Kestrel</string>
  <key>CFBundleIdentifier</key>
  <string>dev.kestrel.ide</string>
  <key>CFBundleExecutable</key>
  <string>Kestrel</string>
  <key>CFBundlePackageType</key>
  <string>APPL</string>
  <key>CFBundleShortVersionString</key>
  <string>0.1.0</string>
  <key>CFBundleVersion</key>
  <string>1</string>
  <key>CFBundleIconFile</key>
  <string>Kestrel</string>
  <key>LSMinimumSystemVersion</key>
  <string>15.0</string>
  <key>LSApplicationCategoryType</key>
  <string>public.app-category.developer-tools</string>
  <key>NSHighResolutionCapable</key>
  <true/>
  <key>NSSupportsAutomaticGraphicsSwitching</key>
  <true/>
  <key>NSHumanReadableCopyright</key>
  <string>Copyright © 2026 Kestrel contributors. Apache-2.0.</string>
</dict>
</plist>
PLIST

IDENTITY="${KESTREL_CODESIGN_IDENTITY:--}"
codesign --force --deep --sign "$IDENTITY" "$APP"
touch "$APP"

echo "Built $APP"
echo "Open with: open \"$APP\""
if [[ "$IDENTITY" == "-" ]]; then
  echo "Signed ad hoc. Set KESTREL_CODESIGN_IDENTITY for Developer ID signing and notarization."
fi
