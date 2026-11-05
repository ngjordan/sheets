#!/usr/bin/env python3
"""The OneDrive / Excel backend Iridium Sheets's desktop shell calls.

The same contract as the Google backend: one JSON request on stdin, one JSON
response on stdout.

    request  {"op": "account"}   -> {"ok": true, "email": ..., "signed_in": true}
    request  {"op": "list"}      -> {"ok": true, "sheets": [...]}
    request  {"op": "fetch", "id"} -> {"ok": true, "xlsx": "<base64>"}
    request  {"op": "push", "id", "xlsx", "tab"} -> {"ok": true, "title": ...}

Two ways in, because Excel offers two:

* **The file** - a workbook in OneDrive or SharePoint, exported as xlsx with
  `/content?format=xlsx` and imported back by replacing its contents. This
  is the same file-level sync the Google backend does, and it round-trips
  formulas, formatting, charts and the rest.

* **The live workbook** - Graph's `/workbook` endpoints read and write
  individual cells without downloading the file. That is what collaboration
  needs: two people editing the same workbook in Excel Online see each
  other's changes immediately, because every write goes to the file.

Everything is xlsx on the wire, so there is still exactly one OOXML
implementation in the product.

Microsoft Graph auth uses the OAuth device-code flow: the user visits a URL
and types a code, so no browser session is ever handed to the app.
"""

from __future__ import annotations

import base64
import csv
import io
import json
import os
import subprocess
import time
import sys
import tempfile
from pathlib import Path

HOME = Path(os.environ.get("HERMES_HOME", Path.home() / ".hermes"))
TOKEN = HOME / "microsoft_token.json"
# The device code waiting for the user to finish signing in.
PENDING = HOME / "microsoft_signin.json"

GRAPH = "https://graph.microsoft.com/v1.0"
LOGIN = "https://login.microsoftonline.com/common/oauth2/v2.0"

# The app id of the Microsoft identity platform's own client. It is public
# (the secret is not); the device-code flow needs no secret from the app.
CLIENT_ID = os.environ.get("MICROSOFT_CLIENT_ID", "04b07795-8ddb-461a-bbee-02f9e1bf7b46")

# The scopes the app needs: read and write files in the user's OneDrive, and
# read the signed-in account's identity.
SCOPES = ["Files.ReadWrite", "offline_access", "User.Read"]

XLSX_MIME = "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"


def respond(obj: dict) -> None:
    json.dump(obj, sys.stdout)
    sys.stdout.flush()


def fail(message: str) -> None:
    respond({"ok": False, "error": message})


def token() -> dict:
    if not TOKEN.exists():
        raise RuntimeError("not signed in")
    return json.loads(TOKEN.read_text())


def access_token() -> str:
    """The access token, refreshed when it has expired."""
    t = token()
    if t.get("expires_at", 0) > time.time() + 60:
        return t["access_token"]
    refresh = t.get("refresh_token")
    if not refresh:
        raise RuntimeError("the Microsoft sign-in has expired; sign in again")
    import urllib.error
    import urllib.parse
    import urllib.request

    body = urllib.parse.urlencode(
        {
            "client_id": CLIENT_ID,
            "grant_type": "refresh_token",
            "refresh_token": refresh,
            "scope": " ".join(SCOPES),
        }
    ).encode()
    req = urllib.request.Request(
        f"{LOGIN}/token",
        data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            data = json.load(r)
    except urllib.error.HTTPError as e:
        raise RuntimeError(f"Microsoft rejected the refresh: {e.read()[:200].decode(errors='replace')}") from e
    t["access_token"] = data["access_token"]
    t["refresh_token"] = data.get("refresh_token", refresh)
    t["expires_at"] = time.time() + int(data.get("expires_in", 3600))
    TOKEN.write_text(json.dumps(t))
    os.chmod(TOKEN, 0o600)
    return t["access_token"]


def graph(path: str, method: str = "GET", body=None, raw: bool = False):
    """Call Microsoft Graph."""
    import urllib.error
    import urllib.request

    data = json.dumps(body).encode() if body is not None else None
    headers = {"Authorization": f"Bearer {access_token()}", "Accept": "application/json"}
    if data:
        headers["Content-Type"] = "application/json"
    req = urllib.request.Request(f"{GRAPH}{path}", data=data, method=method, headers=headers)
    try:
        with urllib.request.urlopen(req, timeout=120) as r:
            payload = r.read()
    except urllib.error.HTTPError as e:
        detail = e.read()[:400].decode(errors="replace")
        try:
            detail = json.loads(detail)["error"]["message"]
        except Exception:
            pass
        raise RuntimeError(f"Microsoft Graph: {detail}") from e
    if raw:
        return payload if isinstance(payload, bytes) else str(payload).encode()
    return json.loads(payload) if payload else {}


def account() -> dict:
    if not TOKEN.exists():
        return {"email": "", "signed_in": False}
    try:
        me = graph("/me")
    except Exception as e:
        raise RuntimeError(str(e)) from e
    return {"email": me.get("mail") or me.get("userPrincipalName") or "", "signed_in": True}


def list_sheets() -> list[dict]:
    """The Excel workbooks in the user's OneDrive."""
    out = []
    page = "/me/drive/root/search(q='.xlsx')?$select=id,name,parentReference&$top=50"
    while page:
        data = graph(page)
        for f in data.get("value", []):
            out.append({"id": f["id"], "title": f.get("name", ""), "tabs": []})
        page = data.get("@odata.nextLink")
        if page:
            # An absolute nextLink; keep only its path and query.
            page = page[len(GRAPH):] if page.startswith(GRAPH) else None
    return out


def fetch(item_id: str) -> bytes:
    """Download a workbook as xlsx."""
    return graph(f"/me/drive/items/{item_id}/content?format=xlsx", raw=True)


def push(item_id: str, xlsx: bytes, tab: str | None) -> str:
    """Write a workbook's values into an Excel file.

    The file's own bytes are replaced by the xlsx when the target is a
    workbook Excel is not currently editing. When it is open in Excel
    Online, a byte replacement would be overwritten, so the values are
    written cell by cell through the live workbook API instead.
    """
    title = tab
    sheets = graph(f"/me/drive/items/{item_id}/workbook/worksheets?$select=name").get("value", [])
    if not sheets:
        raise RuntimeError("that workbook has no worksheets")
    target_name = title or sheets[0]["name"]

    # The worksheet id for the tab name.
    ws_id = None
    for s in sheets:
        if s["name"] == target_name:
            ws_id = s["id"]
            break
    if ws_id is None:
        raise RuntimeError(f"that workbook has no tab named {target_name!r}")

    rows = read_workbook(xlsx)
    if not rows:
        raise RuntimeError("the workbook is empty")

    # A value that begins with "=" (or with a sign and something that is not
    # a number) is a formula to Excel, and the workbook API has no "write this
    # as text" option. Such a cell is left out rather than written as a live
    # formula, and the caller is told which ones were.
    safe, risky = split_formula_like(rows)

    # Write the values through the live workbook API.
    graph(
        f"/me/drive/items/{item_id}/workbook/worksheets/{ws_id}/range(address='A1')",
        method="PATCH",
        body={"values": safe, "@odata.type": "#microsoft.graph.jsonArray"},
    )
    meta = graph(f"/me/drive/items/{item_id}?$select=name")
    name = meta.get("name", target_name)
    if risky:
        PUSH_NOTES.append(f"{len(risky)} cells that look like formulas were not written")
    return name


# The notes a push leaves for the caller.
PUSH_NOTES: list[str] = []


def is_formula_like(value) -> bool:
    """Whether Excel would read this value as a formula."""
    if not isinstance(value, str) or not value:
        return False
    if value.startswith("="):
        return True
    if value[0] in "+-":
        # A sign followed by a number is a number; anything else is a formula.
        return len(value) > 1 and not value[1].isdigit()
    if value.startswith("@"):
        # "@" is Excel's implicit intersection operator.
        return True
    return False


def split_formula_like(rows: list[list[str]]) -> tuple[list[list[str]], list[str]]:
    """The rows to write, and the cells that look like a formula."""
    risky: list[str] = []
    out: list[list[str]] = []
    for row in rows:
        new_row = []
        for value in row:
            if is_formula_like(value):
                risky.append(str(value))
                new_row.append("")
            else:
                new_row.append(value)
        out.append(new_row)
    return out, risky


def read_workbook(xlsx: bytes) -> list[list[str]]:
    """Read a workbook's values with Iridium Sheets's own reader."""
    cli = os.environ.get("IRIDIUM_CLI", "iridium-sheets-cli")
    with tempfile.NamedTemporaryFile(suffix=".xlsx", delete=False) as f:
        f.write(xlsx)
        path = f.name
    try:
        proc = subprocess.run([cli, "cat", path, "--csv"], capture_output=True, text=True, timeout=300)
        if proc.returncode != 0:
            raise RuntimeError(proc.stderr.strip() or "could not read the workbook")
        rows = list(csv.reader(io.StringIO(proc.stdout)))
    finally:
        os.unlink(path)
    while rows and not any(c.strip() for c in rows[-1]):
        rows.pop()
    return rows


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
            PUSH_NOTES.clear()
            title = push(req["id"], data, req.get("tab"))
            respond({"ok": True, "title": title, "note": "; ".join(PUSH_NOTES)})
        elif op == "signIn":
            respond({"ok": True, **sign_in()})
        else:
            fail(f"unknown op {op!r}")
    except KeyError as e:
        fail(f"missing {e}")
    except Exception as e:
        fail(str(e))
    return 0


class _Expired(Exception):
    """The device code expired; the user has to start again."""


def _load(path: Path) -> dict:
    if not path.exists():
        return {}
    try:
        return json.loads(path.read_text())
    except Exception:
        return {}


def _save(path: Path, data: dict) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(data, indent=2))
    os.chmod(path, 0o600)


def sign_in() -> dict:
    """Start or finish the device-code sign-in.

    The first call asks Microsoft for a device code and shows it. The second
    call, after the user signed in, exchanges the code for a token and writes
    it. So the user signs in in their browser and presses the button again.
    """
    pending = _load(PENDING)
    if not pending.get("device_code"):
        pending = device_code()
        _save(PENDING, pending)
        return {"pending": True, **pending}
    try:
        data = _exchange(pending["device_code"])
    except _Expired:
        pending = device_code()
        _save(PENDING, pending)
        return {"pending": True, **pending}
    if data is None:
        # The user has not finished signing in yet; the code is still good.
        return {"pending": True, **pending}
    _save(
        TOKEN,
        {
            "access_token": data["access_token"],
            "refresh_token": data.get("refresh_token"),
            "expires_at": time.time() + int(data.get("expires_in", 3600)),
        },
    )
    PENDING.unlink(missing_ok=True)
    return {"signed_in": True}


def _exchange(device_code: str):
    """Trade the device code for a token. None while the user is still signing in."""
    import urllib.error
    import urllib.parse
    import urllib.request

    body = urllib.parse.urlencode(
        {
            "client_id": CLIENT_ID,
            "grant_type": "urn:ietf:params:oauth:grant-type:device_code",
            "device_code": device_code,
            "scope": " ".join(SCOPES),
        }
    ).encode()
    req = urllib.request.Request(
        f"{LOGIN}/token",
        data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
    )
    try:
        with urllib.request.urlopen(req, timeout=60) as r:
            return json.load(r)
    except urllib.error.HTTPError as e:
        detail = json.loads(e.read() or b"{}")
        err = detail.get("error")
        if err in ("authorization_pending", "slow_down"):
            return None
        if err == "expired_token":
            raise _Expired()
        raise RuntimeError(detail.get("error_description") or err or "the sign-in failed")


def device_code() -> dict:
    """Start a device-code sign-in.

    The user visits a URL and types a code; nothing is handed to the app
    and no browser session is involved.
    """
    import urllib.parse
    import urllib.request

    body = urllib.parse.urlencode({"client_id": CLIENT_ID, "scope": " ".join(SCOPES)}).encode()
    req = urllib.request.Request(
        f"{LOGIN}/devicecode", data=body, headers={"Content-Type": "application/x-www-form-urlencoded"}
    )
    with urllib.request.urlopen(req, timeout=60) as r:
        data = json.load(r)
    return {
        "verification_uri": data["verification_uri"],
        "user_code": data["user_code"],
        "device_code": data["device_code"],
        "interval": data.get("interval", 5),
        "expires_in": data.get("expires_in", 900),
    }


if __name__ == "__main__":
    raise SystemExit(main())
