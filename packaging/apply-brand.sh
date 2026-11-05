#!/usr/bin/env bash
# Apply the branding from branding/brand.env: regenerate every app icon from the
# SVG that file names, and tell you what else to rebuild.
#
#   packaging/apply-brand.sh
#
# Needs resvg (brew install resvg / cargo install resvg) and, on macOS, iconutil
# for the .icns. The icons are committed, so a build never needs these tools.
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# shellcheck source=../branding/brand.env
. "$ROOT/branding/brand.env"

echo "==> $APP_NAME ($APP_ID)"
"$ROOT/packaging/icons.sh"

cat <<EOF

Icons regenerated from $APP_ICON_SVG.

Now rebuild:
  cargo build --release -p $APP_SLUG -p $APP_SLUG-cli

Still hand-maintained (they carry the identity, not the look):
  packaging/linux/flatpak/*.yml       the Flatpak manifest's app id
  packaging/windows/*.wxs              the Windows registry keys and ProgIds
  assets/app-icon/hicolor/scalable     the scalable icon's file name
EOF
