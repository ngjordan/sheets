#!/usr/bin/env python3
"""Tests for the Google connector's push.

A workbook that came from somewhere else can hold text that looks like a
formula. Google reads such a value as a formula, so the connector writes
those cells as text instead.

    python3 tools/test_gsheets_backend.py
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import sys

_spec = importlib.util.spec_from_file_location(
    "gsheets_backend",
    pathlib.Path(__file__).resolve().parent / "gsheets_backend.py",
)
gb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(gb)


def test_a1_addresses_count_from_zero() -> None:
    assert gb.a1(0, 0) == "A1"
    assert gb.a1(0, 25) == "Z1"
    assert gb.a1(0, 26) == "AA1"
    assert gb.a1(2, 1) == "B3"


def test_a_value_that_is_not_formula_like_is_left_alone() -> None:
    rows = [["a", "b"], ["1", "2"]]
    safe, risky = gb.split_formula_like(rows)
    assert risky == []
    assert safe == rows


def test_a_value_that_looks_like_a_formula_is_held_back() -> None:
    rows = [["name", "=SUM(A1:A9)"], ["plain", "text"]]
    safe, risky = gb.split_formula_like(rows)
    assert risky == [("B1", "=SUM(A1:A9)")], risky
    # The bulk write leaves the cell empty, so Google never sees a formula.
    assert safe == [["name", ""], ["plain", "text"]], safe


def test_only_a_leading_equals_marks_a_cell() -> None:
    rows = [["a=b", "b=a", " =c", "x="]]
    _safe, risky = gb.split_formula_like(rows)
    assert risky == [], risky


def test_a_number_that_is_not_a_string_is_left_alone() -> None:
    rows = [[1, 2.5, True, None]]
    safe, risky = gb.split_formula_like(rows)
    assert risky == []
    assert safe == rows


def _stub_cli(rows: str) -> None:
    """Make the push read `rows` instead of calling the CLI."""

    class FakeProc:
        returncode = 0
        stdout = rows
        stderr = ""

    gb.subprocess = type("S", (), {"run": staticmethod(lambda *a, **k: FakeProc())})


def test_the_second_write_uses_raw() -> None:
    """The cells that look like a formula are written with RAW."""
    calls: list[tuple[str, dict]] = []
    rows = [["=1+1"]]
    safe_rows, risky = gb.split_formula_like(rows)
    assert risky == [("A1", "=1+1")]

    def fake_google(path, method="GET", body=None, raw=False, base=""):
        calls.append((path, body or {}))
        if body and body.get("data") and body["data"][0].get("range", "").endswith("!A1"):
            return {"properties": {"title": "Sheet1"}}
        return {"properties": {"title": "Sheet1"}}

    gb.google = fake_google
    _stub_cli("=1+1\n")
    gb.push("sid", b"xlsx-bytes", "Sheet1")
    # Only the writes count; the push reads the title back afterwards.
    writes = [c for c in calls if c[1]]
    assert len(writes) == 2, writes
    bulk, text = writes
    assert bulk[1]["valueInputOption"] == "USER_ENTERED", bulk[1]
    assert bulk[1]["data"][0]["values"] == [[""]], bulk[1]
    assert text[1]["valueInputOption"] == "RAW", text[1]
    assert text[1]["data"] == [{"range": "'Sheet1'!A1", "values": [["=1+1"]]}], text[1]


def test_nothing_extra_is_written_when_no_cell_looks_like_a_formula() -> None:
    calls: list[dict] = []

    def fake_google(path, method="GET", body=None, raw=False, base=""):
        calls.append(body or {})
        return {"properties": {"title": "Sheet1"}}

    gb.google = fake_google
    _stub_cli("hello\n")
    gb.push("sid", b"xlsx-bytes", "Sheet1")
    writes = [c for c in calls if c]
    assert len(writes) == 1, writes
    assert writes[0]["valueInputOption"] == "USER_ENTERED", writes[0]


def test_the_google_sign_in_writes_the_token() -> None:
    """Press Sign in with a client secret file: the token lands where the app looks."""
    import tempfile
    import threading
    import urllib.parse
    import urllib.request
    import webbrowser

    with tempfile.TemporaryDirectory() as d:
        home = pathlib.Path(d)
        gb.HOME = home
        gb.TOKEN = home / "google_token.json"
        gb.CLIENT = home / "google_client_secret.json"
        gb.CLIENT.write_text(json.dumps({"installed": {"client_id": "cid", "client_secret": "secret"}}))

        real_urlopen = urllib.request.urlopen

        def fake_urlopen(req, *a, **k):
            url = getattr(req, "full_url", str(req))
            if "oauth2.googleapis.com/token" in url:
                class R:
                    def __enter__(self):
                        return self

                    def __exit__(self, *a):
                        return False

                    def read(self):
                        return json.dumps({"access_token": "at", "refresh_token": "rt", "expires_in": 3600}).encode()

                return R()
            return real_urlopen(req, *a, **k)

        def fake_open(url):
            # The consent lands on the local port the helper is listening on.
            redirect = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)["redirect_uri"][0]
            threading.Thread(
                target=lambda: real_urlopen(redirect + "?code=the-code", timeout=10)
            ).start()
            return True

        urllib.request.urlopen = fake_urlopen
        webbrowser.open = fake_open
        try:
            out = gb.google_sign_in()
        finally:
            urllib.request.urlopen = real_urlopen
            webbrowser.open = fake_open

        assert out == {"signed_in": True}, out
        token = json.loads(gb.TOKEN.read_text())
        assert token["token"] == "at", token
        assert token["refresh_token"] == "rt", token
        assert token["client_id"] == "cid", token
        assert token["client_secret"] == "secret", token
        assert oct(gb.TOKEN.stat().st_mode)[-3:] == "600", oct(gb.TOKEN.stat().st_mode)


if __name__ == "__main__":
    for fn in [v for k, v in sorted(globals().items()) if k.startswith("test_")]:
        fn()
        print(f"ok  {fn.__name__}")
    print("all tests passed")
