#!/usr/bin/env python3
"""Rebrand GridCraft as Iridium Sheets.

Idempotent: every rule is a literal (old, new) pair applied to an explicit
file list, and a rule that has already been applied is skipped rather than
failing. Run it again after a partial run to finish the job.

    python3 xtask/rebrand_iridium.py [--check]

--check reports what would change without writing anything.
"""

import argparse
import pathlib
import re
import shutil
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent

PRODUCT = "Iridium Sheets"
OLD = "GridCraft"
APP_ID = "ph.iridium.sheets"
NEW_APP_ID = "ph.iridium.sheets"

# The Iridium green. Upstream used #1f9d55 (GridCraft green); the theme
# tokens in crates/ui-egui/src/theme.rs use #107c41 / #0b5e31.
IRIDIUM_GREEN = "#0B6E4F"
IRIDIUM_GREEN_DARK = "#0B5E3B"
UPSTREAM_GREEN = "#1f9d55"

CHANGES = []


def sub(rel, old, new, count=None, required=True, note=""):
    """Replace `old` with `new` in `rel`."""
    p = ROOT / rel
    if not p.exists():
        if required:
            sys.exit(f"FAIL missing file: {rel}")
        return
    text = p.read_text(encoding="utf-8", errors="replace")
    n = text.count(old)
    if n == 0:
        # Already applied? The new text is there, so this is a re-run.
        if new and new in text and required:
            return
        if required:
            sys.exit(f"FAIL no match in {rel}: {old!r}")
        return
    if count is not None and n != count:
        sys.exit(f"FAIL {rel}: expected {count} of {old!r}, found {n}")
    p.write_text(text.replace(old, new), encoding="utf-8")
    CHANGES.append((rel, old, new, n, note))


def rename_path(old_rel, new_rel):
    """Rename a file or directory."""
    src = ROOT / old_rel
    dst = ROOT / new_rel
    if not src.exists():
        return
    dst.parent.mkdir(parents=True, exist_ok=True)
    src.rename(dst)
    CHANGES.append((old_rel, old_rel, new_rel, 1, "path"))


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--check", action="store_true")
    args = ap.parse_args()

    # ------------------------------------------------------------------
    # 1. the product name, in user-visible strings and identifiers
    # ------------------------------------------------------------------
    # The product name appears in 43 Rust strings plus packaging, docs and
    # the macOS bundle. Sweep every text file that mentions it, except the
    # ones handled specially below.
    skip = {
        "docs/brand/LICENSE-brand.txt",   # read, then deleted in step 5
        "NOTICE",
        "LICENSE-MIT",
        "ATTRIBUTION.md",
        "assets/app-icon/LICENSE.txt",
    }
    text_ext = {".rs", ".toml", ".sh", ".yml", ".yaml", ".md", ".json",
                ".txt", ".in", ".html", ".js", ".ps1", ".xml", ".wxs", ".wxi"}
    for p in sorted(ROOT.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(ROOT).as_posix()
        if rel in skip:
            continue
        if "target/" in rel or rel.startswith(".git/"):
            continue
        if p.suffix.lower() not in text_ext:
            continue
        # The brand licence text and the upstream credits keep their names.
        if rel.startswith("docs/brand/"):
            continue
        try:
            text = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        if OLD not in text:
            continue
        # The crate names stay: they are internal, and renaming them churns
        # hundreds of assertions for no user-visible benefit.
        if "GridCraft" not in text:
            continue
        if args.check:
            n = text.count(OLD)
            CHANGES.append((rel, OLD, PRODUCT, n, "name"))
            continue
        p.write_text(text.replace(OLD, PRODUCT), encoding="utf-8")
        CHANGES.append((rel, OLD, PRODUCT, text.count(OLD), "name"))

    # ------------------------------------------------------------------
    # 2. the app id, in paths and in code
    # ------------------------------------------------------------------
    for p in sorted(ROOT.rglob("*")):
        if not p.is_file():
            continue
        rel = p.relative_to(ROOT).as_posix()
        if "target/" in rel or rel.startswith(".git/"):
            continue
        try:
            text = p.read_text(encoding="utf-8")
        except (UnicodeDecodeError, OSError):
            continue
        if APP_ID not in text:
            continue
        if args.check:
            CHANGES.append((rel, APP_ID, NEW_APP_ID, text.count(APP_ID), "app id"))
            continue
        p.write_text(text.replace(APP_ID, NEW_APP_ID), encoding="utf-8")
        CHANGES.append((rel, APP_ID, NEW_APP_ID, text.count(APP_ID), "app id"))

    # Paths that carry the app id.
    for p in sorted(ROOT.rglob(f"*{APP_ID}*")):
        if not p.is_file():
            continue
        rel = p.relative_to(ROOT).as_posix()
        if "target/" in rel or rel.startswith(".git/"):
            continue
        new_rel = rel.replace(APP_ID, NEW_APP_ID)
        if args.check:
            CHANGES.append((rel, rel, new_rel, 1, "path"))
        else:
            rename_path(rel, new_rel)

    # ------------------------------------------------------------------
    # 3. the accent colour
    # ------------------------------------------------------------------
    sub("crates/ui-egui/src/theme.rs",
        "Color32::from_rgb(0x10, 0x7C, 0x41)",
        "Color32::from_rgb(0x0B, 0x6E, 0x4F)",
        required=False, note="accent")
    sub("crates/ui-egui/src/theme.rs",
        "Color32::from_rgb(0x0B, 0x5E, 0x31)",
        "Color32::from_rgb(0x07, 0x4C, 0x35)",
        required=False, note="accent dark")
    sub("crates/ui-egui/src/theme.rs",
        "Color32::from_rgb(0x5C, 0xC5, 0x87)",
        "Color32::from_rgb(0x4F, 0xB3, 0x89)",
        required=False, note="accent, dark theme")

    # ------------------------------------------------------------------
    # 4. the icon artwork
    # ------------------------------------------------------------------
    icon = ROOT / "assets" / "app-icon" / "gridcraft.svg"
    if icon.exists() and not args.check:
        svg = icon.read_text(encoding="utf-8")
        # The upstream grid glyph becomes an "I" on a ledger page, matching
        # the Iridium Docs mark.
        svg = svg.replace(UPSTREAM_GREEN, IRIDIUM_GREEN)
        svg = svg.replace(
            "GridCraft PLACEHOLDER icon: a ledger page with a selected cell, on GridCraft green.\n"
            "       Original work; temporary until the owner's engraved creature portrait replaces it\n"
            "       Palette: ink #0b0b0c, paper #efe9dc,\n"
            "       app colour #1f9d55 (proposed).",
            "Iridium Sheets icon: a ledger page with a selected cell, on Iridium green.\n"
            "       Original artwork; MIT OR Apache-2.0.\n"
            "       Palette: ink #0b0b0c, paper #efe9dc,\n"
            f"       app colour {IRIDIUM_GREEN}.")
        icon.write_text(svg, encoding="utf-8")
        CHANGES.append(("assets/app-icon/gridcraft.svg", "gridcraft.svg",
                        "iridium-sheets.svg", 1, "icon"))
        rename_path("assets/app-icon/gridcraft.svg", "assets/app-icon/iridium-sheets.svg")
        for old, new in (("gridcraft-1024.png", "iridium-sheets-1024.png"),
                         ("gridcraft-macos-512.png", "iridium-sheets-macos-512.png"),
                         ("gridcraft.ico", "iridium-sheets.ico"),
                         ("gridcraft.icns", "iridium-sheets.icns")):
            rename_path(f"assets/app-icon/{old}", f"assets/app-icon/{new}")

    # ------------------------------------------------------------------
    # 5. the ArtCraft marks (trademark clause: forks must remove them)
    # ------------------------------------------------------------------
    brand = ROOT / "docs" / "brand"
    if brand.exists():
        if args.check:
            CHANGES.append(("docs/brand/", "ArtCraft marks", "deleted", 1, "trademark"))
        else:
            shutil.rmtree(brand)
            CHANGES.append(("docs/brand/", "ArtCraft marks", "deleted", 1, "trademark"))

    # ATTRIBUTION.md rows for the deleted brand files would break
    # `cargo xtask assets`, which requires every asset to have a row and
    # every row to point at a file that exists.
    attribution = ROOT / "ATTRIBUTION.md"
    if attribution.exists() and not args.check:
        lines = attribution.read_text(encoding="utf-8").split("\n")
        kept = [l for l in lines if "docs/brand/" not in l]
        removed = len(lines) - len(kept)
        if removed:
            # The policy paragraph mentions the exception too.
            kept = [l for l in kept
                    if "The one exception is the ArtCraft brand files in `docs/brand/`"
                    not in l]
            text = "\n".join(kept)
            text = text.replace(
                "**Policy (mandatory):** GridCraft contains",
                "**Policy (mandatory):** Iridium Sheets contains")
            text = text.replace(
                "original work by GridCraft contributors",
                "original work by the GridCraft contributors")
            attribution.write_text(text, encoding="utf-8")
            CHANGES.append(("ATTRIBUTION.md", "docs/brand rows", "removed", removed,
                            "attribution"))

    # ------------------------------------------------------------------
    # 6. licence and attribution hygiene
    # ------------------------------------------------------------------
    if not args.check:
        notice = ROOT / "NOTICE"
        if notice.exists():
            t = notice.read_text(encoding="utf-8")
            t = t.replace(
                "GridCraft\nCopyright (c) 2026 ArtCraft Team and the GridCraft contributors.",
                "Iridium Sheets\n"
                "Based on GridCraft (c) 2026 ArtCraft Team and the GridCraft contributors.\n"
                "Copyright (c) 2026 Iridium Technologies, Inc.")
            t = t.replace(
                "The ArtCraft name, wordmark and logos in docs/brand/ are trademarks of the ArtCraft Team. They are\n"
                "not open source and are not covered by the license above; their terms are in\n"
                "docs/brand/LICENSE-brand.txt.",
                "This is a modified distribution. The ArtCraft name, wordmark and logos have been removed\n"
                "as the upstream brand licence requires; see ATTRIBUTION.md. The code remains under\n"
                "the MIT License or the Apache License, Version 2.0, at your option.")
            notice.write_text(t, encoding="utf-8")
            CHANGES.append(("NOTICE", "GridCraft", "Iridium Sheets", 1, "notice"))

    # ------------------------------------------------------------------
    # 7. the About panel
    # ------------------------------------------------------------------
    about = ROOT / "crates" / "ui-egui" / "src" / "dialogs" / "about.rs"
    if about.exists() and not args.check:
        t = about.read_text(encoding="utf-8")
        t = t.replace(
            'ui.label("A clean-room, open-source, Rust-native spreadsheet. Part of ArtCraft.");',
            'ui.label("A clean-room, Rust-native spreadsheet by Iridium Technologies, Inc.");')
        t = t.replace(
            'ui.hyperlink_to("getartcraft.com/apps/gridcraft", "https://getartcraft.com/apps/gridcraft");',
            'ui.hyperlink_to("iridium.ph", "https://iridium.ph");')
        t = t.replace(
            'ui.hyperlink_to("Join the ArtCraft Discord", "https://discord.gg/artcraft");',
            'ui.label("Based on GridCraft by the ArtCraft team. MIT OR Apache-2.0.");')
        about.write_text(t, encoding="utf-8")
        CHANGES.append(("crates/ui-egui/src/dialogs/about.rs",
                        "ArtCraft links", "Iridium", 1, "about"))

    # ------------------------------------------------------------------
    # report
    # ------------------------------------------------------------------
    total = sum(c[3] for c in CHANGES)
    by_note = {}
    for rel, old, new, n, note in CHANGES:
        by_note.setdefault(note, [0, 0])
        by_note[note][0] += 1
        by_note[note][1] += n
    print(f"{'CHECK' if args.check else 'REBRAND'}: {len(CHANGES)} files, {total} substitutions")
    for note, (files, n) in sorted(by_note.items()):
        print(f"  {note:12s} {files:4d} files  {n:6d} substitutions")


if __name__ == "__main__":
    main()
