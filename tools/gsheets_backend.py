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

Everything happens in xlsx, the format both sides speak natively, so there
is exactly one OOXML implementation in the product.

The Google APIs are called directly over HTTPS rather than through the
Google API client library: the four calls the app makes are small, and a
stdlib-only helper needs nothing installed beside it. A missing third-party
package inside the app is a failure the user cannot act on.
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
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

HOME = Path(os.environ.get("HERMES_HOME", Path.home() / ".hermes"))
TOKEN = HOME / "google_token.json"

SCOPES = ["https://www.googleapis.com/auth/spreadsheets", "https://www.googleapis.com/auth/drive"]
XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"

DRIVE = "https://www.googleapis.com/drive/v3"
SHEETS = "https://sheets.googleapis.com/v4"
OAUTH = "https://oauth2.googleapis.com"
USERINFO = "https://www.googleapis.com/oauth2/v3/userinfo"


def respond(obj: dict) -> None:
    json.dump(obj, sys.stdout)
    sys.stdout.flush()


def fail(message: str) -> None:
    respond({"ok": False, "error": message})


def token() -> dict:
    if not TOKEN.exists():
        raise RuntimeError("not signed in")
    try:
        return json.loads(TOKEN.read_text())
    except Exception as e:
        raise RuntimeError(f"the Google token file could not be read: {e}") from e


def access_token() -> str:
    """The access token, refreshed when it has expired."""
    t = token()
    # google-auth writes `expiry` as an RFC 3339 timestamp; `expires_at` is
    # what the Microsoft helper writes, so both are understood.
    if t.get("expires_at", 0) > time.time() + 60:
        return t.get("access_token") or t["token"]
    expiry = t.get("expiry")
    if expiry:
        try:
            from datetime import datetime

            when = datetime.fromisoformat(str(expiry).replace("Z", "+00:00")).timestamp()
            if when > time.time() + 60:
                return t["token"]
        except Exception:
            pass
    refresh = t.get("refresh_token")
    if not refresh:
        raise RuntimeError("the Google sign-in has expired; run the Google Workspace setup again")
    body = urllib.parse.urlencode(
        {
            "client_id": t.get("client_id", ""),
            "client_secret": t.get("client_secret", ""),
            "grant_type": "refresh_token",
            "refresh_token": refresh,
        }
    ).encode()
    req = urllib.request.Request(
        f"{OAUTH}/token",
        data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            data = json.load(r)
    except urllib.error.HTTPError as e:
        detail = e.read()[:300].decode(errors="replace")
        raise RuntimeError(f"Google rejected the refresh: {detail}") from e
    t["token"] = data["access_token"]
    t["access_token"] = data["access_token"]
    t["refresh_token"] = data.get("refresh_token", refresh)
    t["expires_at"] = time.time() + int(data.get("expires_in", 3600))
    t.pop("expiry", None)
    TOKEN.write_text(json.dumps(t, indent=2))
    os.chmod(TOKEN, 0o600)
    return t["token"]


def google(path: str, method: str = "GET", body=None, raw: bool = False, base: str = ""):
    """Call a Google API."""
    data = json.dumps(body).encode() if body is not None else None
    headers = {"Authorization": f"Bearer {access_token()}", "Accept": "application/json"}
    if data:
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(f"{base}{path}", data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            payload = r.read()
    except urllib.error.HTTPError as e:
        detail = e.read()[:400].decode(errors="replace")
        try:
            detail = json.loads(detail)["error"]["message"]
        except Exception:
            pass
        raise RuntimeError(f"Google: {detail}") from e
    if raw:
        return payload
    return json.loads(payload) if payload else {}


def account() -> dict:
    if not TOKEN.exists():
        return {"email": "", "signed_in": False}
    try:
        me = google("/userinfo", base=USERINFO)
    except Exception as e:
        raise RuntimeError(str(e)) from e
    return {"email": me.get("email", ""), "signed_in": True}


def list_sheets() -> list[dict]:
    """The spreadsheets the account can open."""
    query = urllib.parse.quote("mimeType='application/vnd.google-apps.spreadsheet' and trashed=false")
    files = google(f"/files?q={query}&fields=files(id,name)&orderBy=modifiedTime desc&pageSize=50", base=DRIVE)
    out = []
    for f in files.get("files", []):
        tabs: list[str] = []
        try:
            meta = google(f"/spreadsheets/{f['id']}", base=SHEETS)
            tabs = [s["properties"]["title"] for s in meta.get("sheets", [])]
        except Exception:
            # A sheet we cannot open is still worth listing; the tabs come back
            # empty.
            pass
        out.append({"id": f["id"], "title": f.get("name", ""), "tabs": tabs})
    return out


def fetch(sid: str) -> bytes:
    """The spreadsheet as an xlsx file."""
    path = f"/files/{sid}/export?" + urllib.parse.urlencode({"mimeType": XLSX_MIME})
    return google(path, raw=True, base=DRIVE)


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

    title = tab
    if not title:
        meta = google(f"/spreadsheets/{sid}", base=SHEETS)
        title = meta["sheets"][0]["properties"]["title"]
    body = {
        "valueInputOption": "USER_ENTERED",
        "data": [{"range": f"'{title}'!A1", "values": rows}],
    }
    google("/spreadsheets/" + sid + "/values:batchUpdate", method="POST", body=body, base=SHEETS)
    meta = google(f"/spreadsheets/{sid}", base=SHEETS)
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
        elif op == "signIn":
            # Google has no in-app sign-in: the token comes from the one-time
            # OAuth setup, which needs the user's own Google Cloud client.
            fail(
                "Sign in to Google with the Google Workspace setup (it asks for your Google Cloud "
                "client, and writes ~/.hermes/google_token.json), then reopen Iridium Sheets."
            )
        else:
            fail(f"unknown op {op!r}")
    except KeyError as e:
        fail(f"missing {e}")
    except Exception as e:
        fail(str(e))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
