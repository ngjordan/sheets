#!/usr/bin/env python3
"""Build a macOS .icns on Linux.

`iconutil` is macOS-only, so packaging/icons.sh cannot produce the .icns here.
The .icns container is a documented Apple format:

    'icns' magic (4 bytes) | total length (4 bytes) | repeated entries:
        type (4 bytes) | entry length (4 bytes) | PNG or JPEG data

The modern form stores PNG data directly in the entry, which is what we do.
Entry types carry the pixel size in the type name, e.g. 'ic07' = 128x128,
'ic08' = 256x256, 'ic09' = 512x512, 'ic10' = 1024x1024 (512@2x).

Usage: make_icns.py <out.icns> <size>:<png> [<size>:<png> ...]
"""

import struct
import sys
from pathlib import Path

# Apple icon type -> pixel size. Types are chosen per size so every PNG lands in
# a slot macOS understands.
TYPE_FOR_SIZE = {
    16: ("icp4", "icp5"),   # 16x16 @1x, @2x
    32: ("icp6", "ic07"),   # 32x32 @1x, @2x (ic07 is also 128)
    64: ("ic08",),          # 64x64 (also used for 128@1x in practice)
    128: ("ic07", "ic08"),  # 128@1x, 128@2x
    256: ("ic09", "ic10"),  # 256@1x (ic09=512), 256@2x
    512: ("ic10",),         # 512@1x
    1024: ("ic11",),        # 1024
}


def build(out: Path, entries: list[tuple[int, Path]]) -> None:
    chunks = []
    used_types: set[str] = set()
    for size, png in entries:
        data = png.read_bytes()
        if data[:8] != b"\x89PNG\r\n\x1a\n":
            raise SystemExit(f"{png}: not a PNG")
        for t in TYPE_FOR_SIZE.get(size, ()):
            if t in used_types:
                continue
            used_types.add(t)
            chunks.append((t, data))
    if not chunks:
        raise SystemExit("no entries")

    body = b"".join(
        t.encode("ascii") + struct.pack(">I", len(data) + 8) + data
        for t, data in chunks
    )
    out.write_bytes(b"icns" + struct.pack(">I", len(body) + 8) + body)
    print(f"wrote {out} ({out.stat().st_size} bytes, {len(chunks)} entries: "
          f"{', '.join(t for t, _ in chunks)})")


def main() -> None:
    if len(sys.argv) < 3:
        raise SystemExit(__doc__)
    out = Path(sys.argv[1])
    entries = []
    for arg in sys.argv[2:]:
        size, _, path = arg.partition(":")
        entries.append((int(size), Path(path)))
    build(out, entries)


if __name__ == "__main__":
    main()
