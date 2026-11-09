#!/usr/bin/env python3
"""The Google Sheets backend Sheets's desktop shell calls.

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
# The OAuth client of the user. Google needs one of their own: there is no
# public client that can read someone's sheets.
CLIENT = HOME / "google_client_secret.json"

SCOPES = ["https://www.googleapis.com/auth/spreadsheets", "https://www.googleapis.com/auth/drive"]
XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"

DRIVE = "https://www.googleapis.com/drive/v3"
SHEETS = "https://sheets.googleapis.com/v4"
OAUTH = "https://oauth2.googleapis.com"
USERINFO = "https://www.googleapis.com/oauth2/v3/userinfo"


def google_sign_in() -> dict:
    """Sign in to Google, with the user's own OAuth client.

    Google has no client an app may borrow: the user creates a "Desktop app"
    client, drops the JSON at ~/.hermes/google_client_secret.json, and this
    runs the whole flow: the consent page opens in their browser, the redirect
    lands on a local port, and the token is written where the app looks for it.
    """
    if not CLIENT.exists():
        raise RuntimeError(
            "Google needs an OAuth client of your own: create a Desktop app client at "
            "https://console.cloud.google.com/apis/credentials with the Sheets and Drive APIs "
            "enabled, download its JSON, and put it at " + str(CLIENT) + ". Then press Sign in again."
        )
    client = json.loads(CLIENT.read_text())
    for key in ("installed", "web", "desktop"):
        if key in client:
            client = client[key]
            break
    cid = client.get("client_id")
    secret = client.get("client_secret")
    if not cid or not secret:
        raise RuntimeError(f"{CLIENT} has no client_id/client_secret")

    import hashlib
    import http.server
    import secrets
    import threading
    import webbrowser

    verifier = base64.urlsafe_b64encode(secrets.token_bytes(48)).rstrip(b"=").decode()
    challenge = base64.urlsafe_b64encode(hashlib.sha256(verifier.encode()).digest()).rstrip(b"=").decode()
    got: dict[str, str] = {}

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            q = urllib.parse.parse_qs(urllib.parse.urlparse(self.path).query)
            got.update({k: v[0] for k, v in q.items()})
            self.send_response(200)
            self.send_header("Content-Type", "text/html")
            self.end_headers()
            self.wfile.write(b"<html><body>Signed in. You can close this tab.</body></html>")

        def log_message(self, *a):
            pass

    srv = http.server.HTTPServer(("127.0.0.1", 0), Handler)
    port = srv.server_address[1]
    threading.Thread(target=srv.serve_forever, daemon=True).start()

    redirect = f"http://127.0.0.1:{port}/"
    url = OAUTH + "/auth?" + urllib.parse.urlencode({
        "client_id": cid,
        "redirect_uri": redirect,
        "response_type": "code",
        "scope": " ".join(SCOPES),
        "code_challenge": challenge,
        "code_challenge_method": "S256",
        "access_type": "offline",
        "prompt": "consent",
    })
    # The URL goes to a file too, in case no browser opens.
    (HOME / "google_signin_url.txt").write_text(url)
    try:
        webbrowser.open(url)
    except Exception:
        pass

    for _ in range(1500):
        if "code" in got or "error" in got:
            break
        threading.Event().wait(0.2)
    srv.shutdown()
    if "error" in got:
        raise RuntimeError(f"the consent was refused: {got.get('error')}")
    if "code" not in got:
        raise RuntimeError(f"no code came back - sign in again in the browser ({url})")

    body = urllib.parse.urlencode({
        "code": got["code"],
        "client_id": cid,
        "client_secret": secret,
        "redirect_uri": redirect,
        "grant_type": "authorization_code",
        "code_verifier": verifier,
    }).encode()
    req = urllib.request.Request(OAUTH + "/token", data=body,
                                 headers={"Content-Type": "application/x-www-form-urlencoded"})
    with urllib.request.urlopen(req, timeout=60) as r:
        data = json.load(r)
    if not data.get("refresh_token"):
        raise RuntimeError("Google gave no refresh token - revoke the app and allow it again")

    TOKEN.parent.mkdir(parents=True, exist_ok=True)
    TOKEN.write_text(json.dumps({
        "token": data["access_token"],
        "refresh_token": data["refresh_token"],
        "client_id": cid,
        "client_secret": secret,
        "scopes": SCOPES,
        "expiry": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime(
            time.time() + int(data.get("expires_in", 3600)))),
        "universe_domain": "googleapis.com",
    }, indent=2))
    os.chmod(TOKEN, 0o600)
    return {"signed_in": True}


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


def a1(row: int, col: int) -> str:
    """A cell's address, counting from zero."""
    letters = ""
    c = col + 1
    while c > 0:
        letters = chr(ord("A") + (c - 1) % 26) + letters
        c = (c - 1) // 26
    return f"{letters}{row + 1}"


def is_formula_like(value) -> bool:
    """Whether Google would read this value as a formula.

    A value that begins with "=" is a formula. So is a sign followed by
    something that is not a number ("-ABS(2)"), and a value that begins with
    "@", which is Excel's implicit intersection operator.
    """
    if not isinstance(value, str) or not value:
        return False
    if value.startswith("="):
        return True
    if value[0] in "+-":
        return len(value) > 1 and not value[1].isdigit()
    if value.startswith("@"):
        return True
    return False


def split_formula_like(rows: list[list[str]]) -> tuple[list[list[str]], list[tuple[str, str]]]:
    """The rows, and the cells whose text Google would read as a formula.

    Google reads a value that begins with "=" as a formula, so a cell that
    holds text like "=SUM(A1)" would arrive in the user's sheet as a live
    formula. Those cells are written separately, as text.
    """
    risky: list[tuple[str, str]] = []
    out: list[list[str]] = []
    for r, row in enumerate(rows):
        new_row = []
        for c, value in enumerate(row):
            if is_formula_like(value):
                risky.append((a1(r, c), value))
                new_row.append("")
            else:
                new_row.append(value)
        out.append(new_row)
    return out, risky


def fetch(sid: str) -> bytes:
    """The spreadsheet as an xlsx file."""
    path = f"/files/{sid}/export?" + urllib.parse.urlencode({"mimeType": XLSX_MIME})
    return google(path, raw=True, base=DRIVE)


def push(sid: str, xlsx: bytes, tab: str | None) -> str:
    """Upload a workbook into a spreadsheet.

    The Sheets API has no "replace this file's contents" call, so the xlsx is
    read here (with Sheets's own reader, through the CLI) and the
    values are written cell by cell. Formulas are sent as USER_ENTERED so
    Google evaluates them rather than storing them as text.
    """
    cli = os.environ.get("IRIDIUM_CLI", "iridium-sheets-cli")
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
    safe_rows, risky = split_formula_like(rows)
    body = {
        "valueInputOption": "USER_ENTERED",
        "data": [{"range": f"'{title}'!A1", "values": safe_rows}],
    }
    google("/spreadsheets/" + sid + "/values:batchUpdate", method="POST", body=body, base=SHEETS)
    if risky:
        # The cells that look like a formula are written as text, so a
        # workbook that arrived from elsewhere cannot put a formula into the
        # user's own sheet.
        google(
            "/spreadsheets/" + sid + "/values:batchUpdate",
            method="POST",
            body={
                "valueInputOption": "RAW",
                "data": [{"range": f"'{title}'!{cell}", "values": [[value]]} for cell, value in risky],
            },
            base=SHEETS,
        )
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
            # A token that works means signed in; otherwise run the OAuth flow
            # with the user's own client.
            try:
                access_token()
            except Exception:
                try:
                    respond({"ok": True, **google_sign_in()})
                except Exception as e:
                    fail(str(e))
                return 0
            respond({"ok": True, "signed_in": True})
        else:
            fail(f"unknown op {op!r}")
    except KeyError as e:
        fail(f"missing {e}")
    except Exception as e:
        fail(str(e))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
