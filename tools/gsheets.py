#!/usr/bin/env python3
"""Bridge Iridium Sheets and Google Sheets.

Google Sheets has no cell-level diff API, so we sync through the format both
sides speak natively: xlsx. The Drive export endpoint is the only xlsx export
Google offers, and Iridium Sheets' own OOXML reader consumes it directly.

    gsheets.py pull  <spreadsheet-id> [--out FILE.xlsx]
    gsheets.py push  <spreadsheet-id> <FILE.xlsx> [--sheet NAME]
    gsheets.py info  <spreadsheet-id>

The local side shells out to ``iridium-sheets-cli`` so there is exactly one OOXML
implementation in the product, not two.
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import os
import subprocess
import sys
from pathlib import Path

HOME = Path(os.environ.get("HERMES_HOME", Path.home() / ".hermes"))
TOKEN = HOME / "google_token.json"
CLI = Path(os.environ.get("IRIDIUM_CLI", "iridium-sheets-cli"))

SCOPES = ["https://www.googleapis.com/auth/spreadsheets", "https://www.googleapis.com/auth/drive"]
XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"


def die(msg: str, code: int = 1) -> None:
    print(f"error: {msg}", file=sys.stderr)
    raise SystemExit(code)


def credentials():
    try:
        from google.oauth2.credentials import Credentials
    except ImportError:
        die(
            "missing google-api-python-client; install it with:\n"
            "  pip install --target /opt/hermes-agent/workspace/pylib "
            "google-api-python-client google-auth-oauthlib google-auth-httplib2"
        )
    if not TOKEN.exists():
        die(f"no Google token at {TOKEN}; run the google-workspace skill's setup first")
    return Credentials.from_authorized_user_file(str(TOKEN), SCOPES)


def cli(*args: str) -> str:
    """Run iridium-sheets-cli and return stdout."""
    proc = subprocess.run([str(CLI), *args], capture_output=True, text=True, timeout=300)
    if proc.returncode != 0:
        die(f"iridium-sheets-cli {' '.join(args)} failed:\n{proc.stderr.strip()}")
    return proc.stdout


def cli_json(*args: str):
    return json.loads(cli(*args))


def local_sheets(path: Path) -> list[dict]:
    """Every sheet in a workbook, with its name and used range.

    ``info --json`` gives names and bounds; ``cat --csv`` gives the values.
    """
    info = cli_json("info", str(path), "--json")
    out = []
    for sheet in info.get("sheets", []):
        out.append({"name": sheet.get("name", ""), "usedRange": sheet.get("usedRange", "")})
    return out


def a1_to_a1(used_range: str) -> tuple[str, str]:
    """Turn iridium-sheets's 'B2:F15' into the A1 range Google wants."""
    if not used_range or ":" not in used_range:
        return "A1", "A1"
    start, end = used_range.split(":", 1)
    return start, end


def read_values(path: Path, sheet: str | None) -> list[list[str]]:
    """Values of one sheet, as a list of row lists."""
    args = ["cat", str(path), "--csv"]
    if sheet:
        args += ["--sheet", sheet]
    raw = cli(*args)
    rows = list(csv.reader(io.StringIO(raw)))
    # Trim trailing empty rows the way the grid itself reports them.
    while rows and not any(c.strip() for c in rows[-1]):
        rows.pop()
    return rows


def main() -> int:
    ap = argparse.ArgumentParser(prog="gsheets", description=__doc__)
    sub = ap.add_subparsers(dest="cmd", required=True)

    p = sub.add_parser("pull", help="download a Google Sheet as xlsx")
    p.add_argument("spreadsheet_id")
    p.add_argument("--out", default="sheet.xlsx")

    p = sub.add_parser("push", help="upload an xlsx into a Google Sheet")
    p.add_argument("spreadsheet_id")
    p.add_argument("file")
    p.add_argument("--sheet", help="target sheet title (default: first sheet in the workbook)")

    p = sub.add_parser("info", help="show a spreadsheet's sheets")
    p.add_argument("spreadsheet_id")

    args = ap.parse_args()
    creds = credentials()

    from googleapiclient.discovery import build

    sheets = build("sheets", "v4", credentials=creds, cache_discovery=False)
    drive = build("drive", "v3", credentials=creds, cache_discovery=False)

    if args.cmd == "info":
        meta = sheets.spreadsheets().get(spreadsheetId=args.spreadsheet_id).execute()
        for s in meta.get("sheets", []):
            props = s["properties"]
            print(f"{props['title']}: gridProperties={props.get('gridProperties', {})}")
        return 0

    if args.cmd == "pull":
        blob = (
            drive.files()
            .export_media(fileId=args.spreadsheet_id, mimeType=XLSX_MIME)
            .execute()
        )
        if not isinstance(blob, bytes):
            blob = io.BytesIO(blob).read()
        out = Path(args.out)
        out.write_bytes(blob)
        print(f"wrote {out} ({len(blob)} bytes)")
        for s in local_sheets(out):
            print(f"  {s['name']}: {s['usedRange']}")
        return 0

    if args.cmd == "push":
        src = Path(args.file)
        if not src.exists():
            die(f"{src} not found")
        sheets_meta = local_sheets(src)
        if not sheets_meta:
            die("the workbook has no sheets")
        title = args.sheet or sheets_meta[0]["name"]
        rows = read_values(src, title if len(sheets_meta) > 1 else None)
        if not rows:
            die(f"'{title}' is empty")

        start, end = a1_to_a1(
            next((s["usedRange"] for s in sheets_meta if s["name"] == title), "")
        )
        body = {
            "valueInputOption": "USER_ENTERED",
            "data": [{"range": f"'{title}'!{start}:{end}", "values": rows}],
        }
        sheets.spreadsheets().values().batchUpdate(
            spreadsheetId=args.spreadsheet_id, body=body
        ).execute()
        print(f"wrote {len(rows)} rows to '{title}' ({start}:{end}) in {args.spreadsheet_id}")
        return 0

    return 1


if __name__ == "__main__":
    raise SystemExit(main())
