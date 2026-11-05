#!/usr/bin/env bash
# Build a macOS installer package from an existing, signed .app bundle.
#
# Usage:
#   ./scripts/bundle-pkg.sh
#   DUNLIN_PKG_SIGN_IDENTITY="Developer ID Installer: ..." ./scripts/bundle-pkg.sh
#
# The app bundle is expected in dist/ (or DUNLIN_DIST_DIR) and is installed to
# /Applications. The package is unsigned unless DUNLIN_PKG_SIGN_IDENTITY names a
# valid "Developer ID Installer" identity.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

APP_NAME="${DUNLIN_APP_NAME:-Dunlin}"
BUNDLE_ID="${DUNLIN_BUNDLE_ID:-dev.dunlin.ide}"
VERSION="${DUNLIN_VERSION:-$(python3 scripts/release_gate.py version)}"
DIST="${DUNLIN_DIST_DIR:-dist}"
APP="$DIST/$APP_NAME.app"
PKG="$DIST/$APP_NAME-$VERSION-macos-arm64.pkg"

python3 scripts/release_gate.py version --value "$VERSION" >/dev/null
python3 - "$APP_NAME" "$BUNDLE_ID" <<'PY'
import re, sys
sys.path.insert(0, 'scripts')
from release_gate import app_name
app_name(sys.argv[1])
if not re.fullmatch(r'[A-Za-z][A-Za-z0-9-]*(?:\.[A-Za-z0-9-]+)+', sys.argv[2]):
    raise SystemExit('Invalid bundle ID')
PY

if [[ ! -d "$APP" ]]; then
  echo "Missing app bundle: $APP (run ./scripts/bundle-macos.sh --build)" >&2
  exit 1
fi
if ! command -v pkgbuild >/dev/null 2>&1; then
  echo "pkgbuild was not found. Install the Xcode command line tools." >&2
  exit 1
fi

codesign --verify --deep --strict "$APP"
rm -f "$PKG"

if [[ -n "${DUNLIN_PKG_SIGN_IDENTITY:-}" ]]; then
  pkgbuild \
    --component "$APP" \
    --install-location /Applications \
    --identifier "$BUNDLE_ID.pkg" \
    --version "$VERSION" \
    --ownership recommended \
    --sign "$DUNLIN_PKG_SIGN_IDENTITY" \
    "$PKG"
else
  pkgbuild \
    --component "$APP" \
    --install-location /Applications \
    --identifier "$BUNDLE_ID.pkg" \
    --version "$VERSION" \
    --ownership recommended \
    "$PKG"
fi

pkgutil --payload-files "$PKG" >/dev/null

echo "Built $PKG"
if [[ -z "${DUNLIN_PKG_SIGN_IDENTITY:-}" ]]; then
  echo "Package is unsigned. Set DUNLIN_PKG_SIGN_IDENTITY for Developer ID Installer signing."
fi
