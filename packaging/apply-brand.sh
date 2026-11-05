#!/usr/bin/env bash
# Apply the branding from branding/brand.env: regenerate every app icon from
# the SVG that file names, and stamp the name and the app id into the files
# the release linter checks as they are.
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

# The desktop entry and the AppStream metadata are validated as they are
# (desktop-file-validate and appstreamcli run on the tracked files), so they
# hold the values rather than placeholders. Stamp them by key, not by value.
DESKTOP="$ROOT/packaging/linux/$APP_ID.desktop"
METAINFO="$ROOT/packaging/linux/$APP_ID.metainfo.xml.in"
if [[ -f "$DESKTOP" ]]; then
  sed -i \
    -e "s|^Name=.*|Name=$APP_NAME|" \
    -e "s|^Exec=.*|Exec=$APP_SLUG %F|" \
    -e "s|^TryExec=.*|TryExec=$APP_SLUG|" \
    -e "s|^Icon=.*|Icon=$APP_ID|" \
    -e "s|^StartupWMClass=.*|StartupWMClass=$APP_ID|" \
    "$DESKTOP"
fi
if [[ -f "$METAINFO" ]]; then
  # Two <name> elements: the app's and the developer's. Python does it in one
  # pass; sed would need the context of each.
  APP_NAME="$APP_NAME" APP_ID="$APP_ID" APP_VENDOR="$APP_VENDOR" python3 - "$METAINFO" <<'PYEOF'
import os, re, sys

path = sys.argv[1]
name = os.environ["APP_NAME"]
app_id = os.environ["APP_ID"]
vendor = os.environ["APP_VENDOR"]
text = open(path, encoding="utf-8").read()
text = re.sub(r"<id>[^<]*</id>", f"<id>{app_id}</id>", text, count=1)
text = re.sub(r"<name>[^<]*</name>", f"<name>{name}</name>", text, count=1)
text = re.sub(r'(<developer id="[^"]*">\s*)<name>[^<]*</name>', rf"\1<name>{vendor}</name>", text)
text = re.sub(r'<developer id="[^"]*">', f'<developer id="{app_id}">', text)
text = re.sub(r'<launchable type="desktop-id">[^<]*</launchable>',
              f'<launchable type="desktop-id">{app_id}.desktop</launchable>', text)
text = re.sub(r'<icon type="stock">[^<]*</icon>', f'<icon type="stock">{app_id}</icon>', text)
text = text.replace(f"{name} is an open-source", f"{name} is an open-source")
open(path, "w", encoding="utf-8").write(text)
PYEOF
fi

"$ROOT/packaging/icons.sh"

cat <<EOF

Icons regenerated from $APP_ICON_SVG.

Now rebuild:
  cargo build --release -p $APP_SLUG -p $APP_SLUG-cli

Still hand-maintained (they carry the identity, not the look):
  packaging/linux/flatpak/*.yml       the Flatpak manifest's app id
  packaging/windows/*.wxs              the Windows registry keys and ProgIds
  the file names of the desktop entry, the metadata and the icons
EOF
