# Branding

`branding/brand.env` is the single source of truth for the app's name, id, vendor and icon.
Everything else reads it:

| Consumer | How it reads it |
|---|---|
| The Rust app (window title, menus, About, messages, PDF creator, MCP server title) | `crates/core/build.rs` reads it at compile time; `iridium_core::brand::{NAME, SLUG, ID, VENDOR, HOMEPAGE}` |
| The Windows resource block (`iridium-sheets.exe` version info) | `apps/iridium-sheets/build.rs` |
| The icons (`packaging/icons.sh`) | `packaging/brand.sh` |
| The Linux desktop entry, AppStream metadata, macOS `Info.plist`, Windows MSI | `packaging/brand.sh` + `@APP_NAME@`-style placeholders |

## Rebranding

1. Edit `branding/brand.env`.
2. Replace the artwork: put your SVG at the path in `APP_ICON_SVG` (keep the
   `viewBox="0 0 512 512"` the macOS layout expects).
3. `packaging/apply-brand.sh` — regenerates every icon size, the `.ico` and the `.icns`.
4. `cargo build --release` — the Rust side picks the new names up at compile time.

## What is identity, not branding

`APP_ID` is the installed identity: the macOS bundle id, the Linux desktop entry name,
the hicolor icon names, the Windows registry keys and the Flatpak app id. It is tied to
the Developer ID signature and to what is already installed on a user's machine, so
change it only for a build you sign and install yourself. Changing it means renaming
the icon files and the `packaging/linux/ph.*` files to match.

`NOTICE` and `ATTRIBUTION.md` name the upstream project on purpose: the MIT and
Apache-2.0 licences require the upstream copyright notice to ship with the code.
