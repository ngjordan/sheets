#!/usr/bin/env python3
"""Sign in to Google for Sheets, from the command line.

Google needs an OAuth client of your own. Create a "Desktop app" client at
https://console.cloud.google.com/apis/credentials with the Sheets and Drive
APIs enabled, download its JSON, and run:

    python3 tools/google_signin.py --client-secret ~/Downloads/client_secret_....json

It does the same thing the app's Sign in button does: the consent page opens,
the redirect is caught, and the token is written to ~/.hermes/google_token.json
where Sheets looks for it.
"""

from __future__ import annotations

import argparse
import importlib.util
import json
import pathlib
import shutil
import sys

spec = importlib.util.spec_from_file_location(
    "gsheets_backend", pathlib.Path(__file__).resolve().parent / "gsheets_backend.py"
)
gb = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gb)


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--client-secret", type=pathlib.Path, required=True)
    args = ap.parse_args()
    if not args.client_secret.exists():
        raise SystemExit(f"{args.client_secret} does not exist")
    gb.CLIENT.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(args.client_secret, gb.CLIENT)
    print(gb.google_sign_in())
    print(f"The token is at {gb.TOKEN}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
