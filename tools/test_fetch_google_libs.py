#!/usr/bin/env python3
"""Tests for the build-time fetcher's integrity check.

The fetcher unpacks third-party Python into the signed application bundle, so
a wheel that does not match the sha256 PyPI published must never be unpacked.

    python3 tools/test_fetch_google_libs.py
"""

from __future__ import annotations

import hashlib
import importlib.util
import io
import shutil
import sys
import tempfile
import zipfile
from pathlib import Path

# The fetcher's filename has a dash, so it is loaded by path rather than
# imported by name.
_spec = importlib.util.spec_from_file_location(
    "fetch_google_libs",
    Path(__file__).resolve().parent.parent / "packaging" / "fetch-google-libs.py",
)
fgl = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(fgl)


class _Resp:
    """A minimal stand-in for the object urlopen returns."""

    def __init__(self, body: bytes):
        self._body = body

    def read(self, size: int = -1) -> bytes:
        if size is None or size < 0:
            out, self._body = self._body, b""
            return out
        out, self._body = self._body[:size], self._body[size:]
        return out

    def __enter__(self):
        return self

    def __exit__(self, *a):
        return False


def _html_response(html: str):
    return _Resp(html.encode())


def _bytes_response(body: bytes):
    return _Resp(body)


def _wheel(payload: bytes) -> bytes:
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w") as z:
        z.writestr("pkg/__init__.py", payload.decode())
    return buf.getvalue()


def test_latest_url_requires_a_hash() -> None:
    """A wheel with no published hash is refused, not fetched blind."""
    html = '<a href="https://files.pythonhosted.org/p/x-1.0-py3-none-any.whl">x</a>'
    orig = fgl.urllib.request.urlopen

    def fake(req, *a, **k):
        return _html_response(html)

    fgl.urllib.request.urlopen = fake
    try:
        fgl.latest_url("x")
    except RuntimeError as e:
        assert "no sha256" in str(e), e
    else:
        raise AssertionError("a wheel with no hash was accepted")
    finally:
        fgl.urllib.request.urlopen = orig


def test_latest_url_returns_the_published_hash() -> None:
    """The hash PyPI publishes comes back with the URL."""
    digest = "a" * 64
    html = (
        f'<a href="https://files.pythonhosted.org/p/x-1.0-py3-none-any.whl#sha256={digest}">x</a>'
    )
    orig = fgl.urllib.request.urlopen

    def fake(req, *a, **k):
        return _html_response(html)

    fgl.urllib.request.urlopen = fake
    try:
        version, url, got = fgl.latest_url("x")
    finally:
        fgl.urllib.request.urlopen = orig
    assert version == "1.0", version
    assert got == digest, got
    assert "#" not in url, url


def test_a_tampered_wheel_is_rejected() -> None:
    """Bytes that do not match the hash are deleted and never unpacked."""
    good = _wheel(b"print('real')")
    digest = hashlib.sha256(good).hexdigest()
    dest = Path(tempfile.mkdtemp())
    try:
        # Serve different bytes than the one the hash was computed over.
        def fake_urlopen(req, *a, **k):
            return _bytes_response(_wheel(b"print('tampered')"))

        orig_urlopen, orig_latest = fgl.urllib.request.urlopen, fgl.latest_url
        fgl.urllib.request.urlopen = fake_urlopen
        fgl.latest_url = lambda name: ("1.0", "https://example.invalid/x-1.0-py3-none-any.whl", digest)
        try:
            fgl.fetch_one("x", dest)
        except RuntimeError as e:
            assert "mismatch" in str(e), e
        else:
            raise AssertionError("a tampered wheel was unpacked")
        finally:
            fgl.urllib.request.urlopen, fgl.latest_url = orig_urlopen, orig_latest
        # Nothing was unpacked and the temporary wheel is gone.
        assert not list(dest.glob("pkg")), list(dest.iterdir())
        assert not list(dest.glob("*.whl")), list(dest.iterdir())
    finally:
        shutil.rmtree(dest, ignore_errors=True)


def test_a_matching_wheel_is_unpacked() -> None:
    """The normal path still works."""
    good = _wheel(b"print('real')")
    digest = hashlib.sha256(good).hexdigest()
    dest = Path(tempfile.mkdtemp())
    try:
        def fake_urlopen(req, *a, **k):
            return _bytes_response(good)

        orig_urlopen, orig_latest = fgl.urllib.request.urlopen, fgl.latest_url
        fgl.urllib.request.urlopen = fake_urlopen
        fgl.latest_url = lambda name: ("1.0", "https://example.invalid/x-1.0-py3-none-any.whl", digest)
        try:
            fgl.fetch_one("x", dest)
        finally:
            fgl.urllib.request.urlopen, fgl.latest_url = orig_urlopen, orig_latest
        assert (dest / "pkg" / "__init__.py").read_text() == "print('real')"
    finally:
        shutil.rmtree(dest, ignore_errors=True)


if __name__ == "__main__":
    for fn in (
        test_latest_url_requires_a_hash,
        test_latest_url_returns_the_published_hash,
        test_a_tampered_wheel_is_rejected,
        test_a_matching_wheel_is_unpacked,
    ):
        fn()
        print(f"ok  {fn.__name__}")
    print("all tests passed")
