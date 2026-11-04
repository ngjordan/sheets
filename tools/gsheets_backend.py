#!/usr/bin/env python3
"""The Google Sheets backend Iridium Sheets's desktop shell calls.

One JSON request on stdin, one JSON response on stdout. The Rust side
runs this on a worker thread, so it must never block on the UI and must
never print anything but the response.

    request  {"op": "account"}
    response {"ok": true, "email": "...", "signed_in": true}

    request  {"op": "list"}
    response {"ok": true, "sheets": [{"id": "...", "title": "...", "tabs": ["Sheet1"]}]}

    request  {"op": "fetch", "id": "..."}
    response {"ok": true, "xlsx": "<base64>"}

    request  {"op": "push", "id": "...", "xlsx": "<base64>", "tab": "Sheet1"}
    response {"ok": true, "title": "..."}

Everything happens in xlsx, the format both sides speak natively, so there is
exactly one OOXML implementation in the product.
"""

from __future__ import annotations

import base64
import csv
import io
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path

HOME = Path(os.environ.get("HERMES_HOME", Path.home() / ".hermes"))
TOKEN = HOME / "google_token.json"
SCOPES = ["https://www.googleapis.com/auth/spreadsheets", "https://www.googleapis.com/auth/drive"]
XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"


def respond(obj: dict) -> None:
    json.dump(obj, sys.stdout)
    sys.stdout.flush()


def fail(message: str) -> None:
    respond({"ok": False, "error": message})


def credentials():
    try:
        from google.oauth2.credentials import Credentials
    except ImportError:
        raise RuntimeError(
            "Google support needs the Google API libraries. Install them with:\n"
            "  pip install --target /opt/hermes-agent/workspace/pylib "
            "google-api-python-client google-auth-oauthlib google-auth-httplib2"
        )
    if not TOKEN.exists():
        raise RuntimeError("not signed in")
    return Credentials.from_authorized_user_file(str(TOKEN), SCOPES)


def services():
    from googleapiclient.discovery import build

    creds = credentials()
    return (
        build("sheets", "v4", credentials=creds, cache_discovery=False),
        build("drive", "v3", credentials=creds, cache_discovery=False),
    )


def account() -> dict:
    if not TOKEN.exists():
        return {"email": "", "signed_in": False}
    try:
        creds = credentials()
    except Exception as e:
        raise RuntimeError(str(e)) from e
    email = getattr(creds, "account_name", "") or ""
    if not email:
        # The token file keeps the account it was minted for.
        try:
            email = json.loads(TOKEN.read_text()).get("account_name", "") or ""
        except Exception:
            email = ""
    return {"email": email, "signed_in": True}


def list_sheets() -> list[dict]:
    sheets, drive = services()
    # The Drive API is what can search by name; the Sheets API lists a file's tabs.
    files = (
        drive.files()
        .list(
            q="mimeType='application/vnd.google-apps.spreadsheet' and trashed=false",
            fields="files(id,name)",
            orderBy="modifiedTime desc",
            pageSize=50,
        )
        .execute()
    )
    out = []
    for f in files.get("files", []):
        tabs: list[str] = []
        try:
            meta = sheets.spreadsheets().get(spreadsheetId=f["id"]).execute()
            tabs = [s["properties"]["title"] for s in meta.get("sheets", [])]
        except Exception:
            # A sheet we cannot open is still worth listing; the tabs come back empty.
            pass
        out.append({"id": f["id"], "title": f.get("name", ""), "tabs": tabs})
    return out


def fetch(sid: str) -> bytes:
    _, drive = services()
    blob = drive.files().export_media(fileId=sid, mimeType=XLSX_MIME).execute()
    return blob if isinstance(blob, bytes) else io.BytesIO(blob).read()


def push(sid: str, xlsx: bytes, tab: str | None) -> str:
    """Upload a workbook into a spreadsheet.

    The Sheets API has no "replace this file's contents" call, so the xlsx is
    read here (with Iridium Sheets's own reader, through the CLI) and the
    values are written cell by cell. Formulas are sent as USER_ENTERED so
    Google evaluates them rather than storing them as text.
    """
    cli = os.environ.get("GRIDCRAFT_CLI", "gridcraft-cli")
    with tempfile.NamedTemporaryFile(suffix=".xlsx", delete=False) as f:
        f.write(xlsx)
        path = f.name
    try:
        args = [cli, "cat", path, "--csv"]
        if tab:
            args += ["--sheet", tab]
        proc = subprocess.run(args, capture_output=True, text=True, timeout=300)
        if proc.returncode != 0:
            raise RuntimeError(proc.stderr.strip() or "could not read the workbook")
        rows = list(csv.reader(io.StringIO(proc.stdout)))
    finally:
        os.unlink(path)
    while rows and not any(c.strip() for c in rows[-1]):
        rows.pop()
    if not rows:
        raise RuntimeError("the workbook is empty")

    sheets, _ = services()
    title = tab
    if not title:
        meta = sheets.spreadsheets().get(spreadsheetId=sid).execute()
        title = meta["sheets"][0]["properties"]["title"]
    body = {
        "valueInputOption": "USER_ENTERED",
        "data": [{"range": f"'{title}'!A1", "values": rows}],
    }
    sheets.spreadsheets().values().batchUpdate(spreadsheetId=sid, body=body).execute()
    meta = sheets.spreadsheets().get(spreadsheetId=sid).execute()
    return meta.get("properties", {}).get("title", title)


def main() -> int:
    try:
        req = json.loads(sys.stdin.read() or "{}")
    except Exception as e:
        fail(f"bad request: {e}")
        return 1
    op = req.get("op")
    try:
        if op == "account":
            respond({"ok": True, **account()})
        elif op == "list":
            respond({"ok": True, "sheets": list_sheets()})
        elif op == "fetch":
            data = base64.b64decode(req["xlsx"]) if "xlsx" in req else fetch(req["id"])
            respond({"ok": True, "xlsx": base64.b64encode(data).decode()})
        elif op == "push":
            data = base64.b64decode(req["xlsx"])
            title = push(req["id"], data, req.get("tab"))
            respond({"ok": True, "title": title})
        else:
            fail(f"unknown op {op!r}")
    except KeyError as e:
        fail(f"missing {e}")
    except Exception as e:
        fail(str(e))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
