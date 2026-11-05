#!/usr/bin/env bash
# Publish verified release artifacts to an OCI registry (GHCR by default).
#
# Usage:
#   ./scripts/publish-oci.sh [VERSION]
#   DUNLIN_OCI_REPOSITORY=ghcr.io/corapoid/dunlin ./scripts/publish-oci.sh 0.1.0-beta.8
#
# Requires oras (https://oras.land) and an authenticated registry session.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

VERSION="${1:-$(python3 scripts/release_gate.py version)}"
APP_NAME="${DUNLIN_APP_NAME:-Dunlin}"
REPOSITORY="${DUNLIN_OCI_REPOSITORY:-ghcr.io/corapoid/dunlin}"
DIST="${DUNLIN_DIST_DIR:-dist}"
REF="$REPOSITORY:$VERSION"

python3 scripts/release_gate.py version --value "$VERSION" >/dev/null
python3 - "$APP_NAME" <<'PY'
import sys
sys.path.insert(0, 'scripts')
from release_gate import app_name
app_name(sys.argv[1])
PY

if ! command -v oras >/dev/null 2>&1; then
  echo "oras was not found. Install it from https://oras.land." >&2
  exit 1
fi

ZIP="$DIST/$APP_NAME-$VERSION-macos-arm64.zip"
DMG="$DIST/$APP_NAME-$VERSION-macos-arm64.dmg"
PKG="$DIST/$APP_NAME-$VERSION-macos-arm64.pkg"
RELAY="$DIST/dunlin-relay-$VERSION-macos-arm64.tar.gz"
SUMS="$DIST/SHA256SUMS"
for artifact in "$ZIP" "$DMG" "$PKG" "$RELAY" "$SUMS"; do
  [[ -f "$artifact" ]] || {
    echo "Missing release artifact: $artifact" >&2
    exit 1
  }
done

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
cat > "$TMP/INSTALL.md" <<EOF
# Dunlin $VERSION

Native, agent-first IDE for macOS 15+ on Apple Silicon. This release is a
public beta and the app is ad-hoc signed, not Apple-notarized.

## Pull

\`\`\`sh
oras pull $REF -o dunlin-$VERSION
\`\`\`

Then open \`dunlin-$VERSION/$APP_NAME-$VERSION-macos-arm64.dmg\`, or run the
\`.pkg\` installer. Verify every file against \`SHA256SUMS\`.

Homebrew beta channel:

\`\`\`sh
brew trust corapoid/tap
brew install --cask corapoid/tap/dunlin@beta
\`\`\`

Project: https://github.com/corapoid/dunlin
EOF

oras push "$REF" \
  --artifact-type "application/vnd.dunlin.bundle.v1+json" \
  --annotation "org.opencontainers.image.title=$APP_NAME $VERSION" \
  --annotation "org.opencontainers.image.description=Native, agent-first IDE for macOS (public beta)" \
  --annotation "org.opencontainers.image.version=$VERSION" \
  --annotation "org.opencontainers.image.source=https://github.com/corapoid/dunlin" \
  --annotation "org.opencontainers.image.url=https://github.com/corapoid/dunlin" \
  --annotation "org.opencontainers.image.documentation=https://github.com/corapoid/dunlin/blob/main/docs/guide.md" \
  --annotation "org.opencontainers.image.licenses=MIT" \
  "$DMG:application/x-apple-diskimage" \
  "$PKG:application/vnd.apple.installer+xml" \
  "$ZIP:application/zip" \
  "$RELAY:application/gzip" \
  "$SUMS:text/plain" \
  "$TMP/INSTALL.md:text/markdown"

if [[ "$VERSION" == *-* ]]; then
  oras tag "$REF" beta
else
  oras tag "$REF" latest
fi

echo "Published $REF"
