#!/usr/bin/env python3
"""The connector backend Iridium Sheets's desktop shell calls.

One JSON request on stdin, one JSON response on stdout, the same shape as
the other cloud helpers. The Rust side runs this on a worker thread, so it
must never block on the UI and must never print anything but the response.

    request  {"op": "list"}
    response {"ok": true, "connectors": [{"name": "..."}]}

    request  {"op": "fetch", "connector": {...}}
    response {"ok": true, "header": ["..."], "rows": [["..."]], "note": "..."}

    request  {"op": "save", "connector": {...}}
    response {"ok": true}

    request  {"op": "remove", "name": "..."}
    response {"ok": true}

Connectors are kept in a file beside the user's own settings, so a connector
the user set up is still there next time. The file holds the name, the URL and
the headers; a header whose value starts with ``env:`` is replaced by that
environment variable's value when the request is made, so a secret never
reaches the disk.
"""

from __future__ import annotations

import json
import os
import sys
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path

HOME = Path(os.environ.get("HERMES_HOME", Path.home() / ".hermes"))
STORE = HOME / "connectors.json"

# A request may not run forever: a server that never answers must not hang
# the sheet.
TIMEOUT = 60

# A reply bigger than this is refused rather than loaded into memory.
MAX_BYTES = 32 * 1024 * 1024


def respond(obj: dict) -> None:
    json.dump(obj, sys.stdout)
    sys.stdout.flush()


def fail(message: str) -> None:
    respond({"ok": False, "error": message})


def load() -> list[dict]:
    if not STORE.exists():
        return []
    try:
        data = json.loads(STORE.read_text())
    except Exception:
        return []
    return data if isinstance(data, list) else []


def save_all(connectors: list[dict]) -> None:
    """Write the store atomically, so a crash cannot leave it empty."""
    STORE.parent.mkdir(parents=True, exist_ok=True)
    tmp = STORE.with_name(f"{STORE.name}.{os.getpid()}.tmp")
    try:
        # The file holds credentials, so only the user may read it.
        fd = os.open(str(tmp), os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(fd, "w") as f:
            json.dump(connectors, f, indent=1)
        os.replace(tmp, STORE)
        os.chmod(STORE, 0o600)
    finally:
        if tmp.exists():
            tmp.unlink()


def resolved_headers(connector: dict, url: str) -> tuple[dict[str, str], list[str]]:
    """The headers to send, and the notes about the ones that were left out.

    A header whose value is ``env:NAME@host`` reads that environment variable
    and sends it, but only to ``host``: the secret follows the host it was
    saved for, so a connector that is pointed somewhere else never carries it.
    A plain ``env:NAME`` is refused, and so is any ``env:`` header on a
    connector that is not a saved one (the store is the user's own list).
    """
    headers: dict[str, str] = {}
    notes: list[str] = []
    saved = connector.get("saved") is True
    for pair in connector.get("headers") or []:
        if not isinstance(pair, (list, tuple)) or len(pair) != 2:
            continue
        key, value = str(pair[0]), str(pair[1])
        if value.startswith("env:"):
            spec = value[4:]
            name, _, want_host = spec.partition("@")
            if not want_host:
                notes.append(f"{key}: an env: header needs a host (env:NAME@host)")
                continue
            if not saved:
                notes.append(f"{key}: an env: header only works on a saved connector")
                continue
            if host_of(url) != want_host.lower():
                notes.append(f"{key}: env: header not sent to {host_of(url)}")
                continue
            value = os.environ.get(name, "")
            if not value:
                notes.append(f"{key}: {name} is not set")
                continue
        headers[key] = value
    return headers, notes


def host_of(url: str) -> str:
    """The host of a URL, lowercased, without the port."""
    host = urllib.parse.urlsplit(url).hostname or ""
    return host.lower()


def fetch(connector: dict) -> tuple[list[str], list[list[str]], str]:
    """The rows a connector holds, and a note about the fetch."""
    url = str(connector.get("url", "")).strip()
    if not url:
        raise RuntimeError("the connector has no URL")
    if not url.startswith(("http://", "https://")):
        # Only the network is a connector source. A local file would let any
        # caller read one, and the file open already handles those.
        raise RuntimeError("a connector's URL must be http:// or https://")

    method = str(connector.get("method") or "GET").upper()
    if method not in ("GET", "POST", "HEAD"):
        raise RuntimeError(f"{method} is not a connector method; use GET, POST or HEAD")
    body = None
    headers, notes = resolved_headers(connector, url)
    headers.setdefault("User-Agent", "iridium-sheets")
    headers.setdefault("Accept", "application/json, text/csv, */*")

    request = urllib.request.Request(url, method=method, data=body)
    # `add_unredirected_header` keeps a header off a redirect: a server that
    # answers 302 sends the secret only to the host it was meant for.
    for key, value in headers.items():
        request.add_unredirected_header(key, value)
    try:
        with urllib.request.urlopen(request, timeout=TIMEOUT) as response:
            raw = response.read(MAX_BYTES + 1)
            if len(raw) > MAX_BYTES:
                raise RuntimeError(f"the server sent more than {MAX_BYTES} bytes")
            text = raw.decode("utf-8", "replace")
    except urllib.error.HTTPError as e:
        detail = e.read().decode("utf-8", "replace")[:200] if e.fp else ""
        raise RuntimeError(f"the server answered {e.code} {e.reason}{': ' + detail if detail else ''}") from e
    except urllib.error.URLError as e:
        raise RuntimeError(f"could not reach {url}: {e.reason}") from e
    except TimeoutError as e:
        raise RuntimeError(f"{url} did not answer within {TIMEOUT} seconds") from e
    header, rows, note = parse_rows(text, connector)
    if notes:
        note = f"{note}; " + "; ".join(notes)
    return header, rows, note


def parse_rows(text: str, connector: dict) -> tuple[list[str], list[list[str]], str]:
    """The reply as a header and rows."""
    path = str(connector.get("path") or "")
    source = str(connector.get("source") or "rest")
    stripped = text.lstrip()
    if source == "csv" or (source == "rest" and not stripped.startswith(("{", "["))):
        return rows_from_csv(text)
    return rows_from_json(text, path)


def rows_from_json(reply: str, path: str) -> tuple[list[str], list[list[str]], str]:
    value = json.loads(reply)
    if path:
        for segment in [s for s in path.split(".") if s]:
            if not isinstance(value, dict) or segment not in value:
                raise RuntimeError(f"the reply has no '{path}'")
            value = value[segment]
    if isinstance(value, dict):
        value = [value]
    if not isinstance(value, list):
        raise RuntimeError("the reply is not a list of records")
    header: list[str] = []
    for item in value:
        if isinstance(item, dict):
            for key in item:
                if key not in header:
                    header.append(key)
    rows = []
    for item in value:
        if not isinstance(item, dict):
            rows.append([scalar(item)])
            continue
        rows.append([scalar(item.get(k, "")) for k in header])
    return header, rows, f"{len(rows)} rows"


def rows_from_csv(text: str) -> tuple[list[str], list[list[str]], str]:
    import csv
    import io

    records = list(csv.reader(io.StringIO(text)))
    if not records:
        return [], [], "0 rows"
    header = records[0]
    rows = []
    for record in records[1:]:
        record = (record + [""] * len(header))[: len(header)]
        rows.append(record)
    return header, rows, f"{len(rows)} rows"


def scalar(value) -> str:
    if value is None:
        return ""
    if isinstance(value, bool):
        return "TRUE" if value else "FALSE"
    if isinstance(value, str):
        return value
    if isinstance(value, (int, float)):
        return repr(value) if isinstance(value, float) else str(value)
    return json.dumps(value)


def main() -> int:
    try:
        req = json.loads(sys.stdin.read() or "{}")
    except Exception as e:
        fail(f"bad request: {e}")
        return 1
    op = req.get("op")
    try:
        if op == "list":
            respond({"ok": True, "connectors": load()})
        elif op == "fetch":
            connector = req.get("connector") or {}
            header, rows, note = fetch(connector)
            respond({"ok": True, "header": header, "rows": rows, "note": note})
        elif op == "save":
            connector = req.get("connector") or {}
            name = str(connector.get("name", "")).strip()
            if not name:
                fail("the connector needs a name")
                return 1
            connectors = [c for c in load() if str(c.get("name", "")) != name]
            connectors.append(connector)
            save_all(connectors)
            respond({"ok": True})
        elif op == "remove":
            name = str(req.get("name", ""))
            save_all([c for c in load() if str(c.get("name", "")) != name])
            respond({"ok": True})
        else:
            fail(f"unknown op {op!r}")
    except Exception as e:
        fail(str(e))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
