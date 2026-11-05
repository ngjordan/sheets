#!/usr/bin/env python3
"""The cloud helpers must run on the standard library alone.

They ship inside the application bundle, where a missing third-party package
shows up as "No module named 'pyparsing'" in front of the user and cannot be
fixed from there. These checks fail the build instead.

    python3 tools/test_helpers_are_stdlib_only.py
"""

from __future__ import annotations

import pathlib
import subprocess
import sys

TOOLS = pathlib.Path(__file__).resolve().parent
HELPERS = ["gsheets_backend.py", "m365_backend.py", "connector_backend.py"]

# The site-packages directories are removed from the path, so nothing but the
# standard library is importable.
ISOLATE = (
    "import sys;"
    "sys.path = [p for p in sys.path if 'site-packages' not in p and 'dist-packages' not in p];"
)


def _run(code: str) -> subprocess.CompletedProcess:
    return subprocess.run(
        [sys.executable, "-c", code], capture_output=True, text=True, timeout=120
    )


def test_each_helper_imports_without_third_party_packages() -> None:
    """Every helper loads with only the standard library."""
    for helper in HELPERS:
        path = TOOLS / helper
        code = ISOLATE + f"exec(open({str(path)!r}).read().replace('__main__', '__notmain__'))"
        proc = _run(code)
        assert proc.returncode == 0, f"{helper} needs a third-party package:\n{proc.stderr[-800:]}"


def test_each_helper_answers_a_request_without_third_party_packages() -> None:
    """Every helper answers a request with only the standard library."""
    for helper in HELPERS:
        proc = subprocess.run(
            [sys.executable, str(TOOLS / helper)],
            input=b'{"op":"account"}',
            capture_output=True,
            timeout=120,
        )
        out = proc.stdout.decode(errors="replace").strip()
        assert proc.returncode == 0, f"{helper} failed: {proc.stderr.decode(errors='replace')[-500:]}"
        assert out.startswith("{"), f"{helper} printed no JSON: {out[:200]!r}"
        assert '"ok"' in out, f"{helper} printed no answer: {out[:200]!r}"


def test_the_google_helper_needs_no_google_libraries() -> None:
    """The Google helper must not import the Google client library.

    That library pulls in cryptography and opentelemetry, which have to be
    fetched as platform wheels and then have to be present at run time.
    """
    path = TOOLS / "gsheets_backend.py"
    # The names are compared against a snapshot taken before the helper runs:
    # the interpreter may already hold a namespace package from a .pth file,
    # which is not the helper importing anything.
    code = (
        ISOLATE
        + "import sys;before=set(sys.modules);"
        + f"exec(open({str(path)!r}).read().replace('__main__', '__notmain__'));"
        + "new=set(sys.modules)-before;"
        + "bad=[m for m in new if m.split('.')[0] in "
        + "('googleapiclient','google','google_auth_oauthlib','cryptography','pyparsing','oauthlib','opentelemetry')];"
        + "print(','.join(sorted(bad)));"
        + "raise SystemExit(1 if bad else 0)"
    )
    proc = _run(code)
    assert proc.returncode == 0, f"the Google helper imports third-party code: {proc.stdout.strip()}"


if __name__ == "__main__":
    for fn in [v for k, v in sorted(globals().items()) if k.startswith("test_")]:
        fn()
        print(f"ok  {fn.__name__}")
    print("all tests passed")
