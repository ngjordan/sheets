#!/usr/bin/env python3
"""Tests for the Excel connector's sign-in.

Microsoft uses the device-code flow: the app asks for a code, the user signs
in in their browser, and the code is then traded for a token. The button does
both halves, so the user presses it again after signing in.

    python3 tools/test_m365_backend.py
"""

from __future__ import annotations

import importlib.util
import json
import pathlib
import tempfile

_spec = importlib.util.spec_from_file_location(
    "m365_backend",
    pathlib.Path(__file__).resolve().parent / "m365_backend.py",
)
mb = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(mb)


def test_the_sign_in_takes_two_presses() -> None:
    with tempfile.TemporaryDirectory() as d:
        home = pathlib.Path(d)
        mb.HOME = home
        mb.TOKEN = home / "microsoft_token.json"
        mb.PENDING = home / "microsoft_signin.json"

        mb.device_code = lambda: {
            "verification_uri": "https://login.microsoft.com/device",
            "user_code": "DE3QDRN54",
            "device_code": "the-device-code",
            "interval": 5,
            "expires_in": 900,
        }
        exchanged: list[str] = []

        def fake_exchange(code: str):
            exchanged.append(code)
            if len(exchanged) == 1:
                return None  # the user has not signed in yet
            return {"access_token": "the-token", "refresh_token": "the-refresh", "expires_in": 3600}

        mb._exchange = fake_exchange

        # The first press asks Microsoft for a code and shows it.
        first = mb.sign_in()
        assert first["pending"] is True, first
        assert first["user_code"] == "DE3QDRN54", first
        assert not mb.TOKEN.exists(), "no token before the user signs in"

        # The second press asks Microsoft, which says the user has not signed in
        # yet, so the same code stays good.
        second = mb.sign_in()
        assert second.get("pending") is True, second
        assert second["user_code"] == "DE3QDRN54", second

        # The third press, after the user signed in, exchanges the code.
        third = mb.sign_in()
        assert third.get("signed_in") is True, third
        token = json.loads(mb.TOKEN.read_text())
        assert token["access_token"] == "the-token", token
        assert token["refresh_token"] == "the-refresh", token
        assert not mb.PENDING.exists(), "the pending code is cleared"


if __name__ == "__main__":
    for fn in [v for k, v in sorted(globals().items()) if k.startswith("test_")]:
        fn()
        print(f"ok  {fn.__name__}")
    print("all tests passed")
