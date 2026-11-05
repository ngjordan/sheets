#!/usr/bin/env python3
"""Fetch the Google API libraries for the Hermes-side Google bridge.

The application itself no longer needs these: its Google helper talks to the
Google APIs over HTTPS with only the standard library, so nothing is
fetched at build time and no third-party code is shipped in the bundle.

The bridge in `tools/gsheets.py` still uses the Google API client, and the
workspace scripts that use it need the libraries on their path, so they are
fetched here rather than committed - the same arrangement the project uses
for its fonts.

Every download is checked against the sha256 PyPI publishes for that exact
file, so a wheel that does not match is never unpacked.

The set is deliberately small: the Google API client, its auth helpers and
their dependencies, plus the two discovery documents the app actually uses
(Sheets and Drive). The full discovery cache is ~109 MB of other APIs'
schemas and is not needed.

    python3 packaging/fetch-google-libs.py <target-dir>

The target directory ends up holding the packages to put on PYTHONPATH.
"""

from __future__ import annotations

import hashlib
import os
import re
import shutil
import sys
import urllib.parse
import urllib.request
import zipfile
from pathlib import Path

# The distributions the helper needs, and the import names they provide.
#
# The distribution name is not always the import name: the Google API client
# is imported as `googleapiclient` but published as `google-api-python-client`,
# and PyPI's simple index only answers for the published name.
PACKAGES = {
    "google-api-python-client": "googleapiclient",
    "google-auth": "google",
    "google-auth-oauthlib": "google_auth_oauthlib",
    "httplib2": "httplib2",
    "uritemplate": "uritemplate",
    "requests": "requests",
    "requests-oauthlib": "requests_oauthlib",
    "urllib3": "urllib3",
    "charset-normalizer": "charset_normalizer",
    "idna": "idna",
    "certifi": "certifi",
    "pyasn1": "pyasn1",
    "pyasn1-modules": "pyasn1_modules",
    "google-auth-httplib2": "google_auth_httplib2",
    "google-api-core": "google.api_core",
    "googleapis-common-protos": "google.api_core",
    "proto-plus": "proto",
    "protobuf": "google.protobuf",
    # The hard dependencies of the packages above. Without them the import
    # fails inside the app, where the message is "No module named ..." and
    # the user cannot tell what to do.
    "oauthlib": "oauthlib",
    "pyparsing": "pyparsing",
    "opentelemetry-api": "opentelemetry",
    "opentelemetry-sdk": "opentelemetry.sdk",
    "opentelemetry-semantic-conventions": "opentelemetry.semconv",
}

# The discovery documents the app uses. Anything else in the cache is another
# API's schema, which the app never calls.
DISCOVERY = ["sheets.v4.json", "drive.v3.json", "drive.v2.json", "driveactivity.v2.json", "drivelabels.v2.json"]

# Where the wheels come from. PyPI's simple index lists the files directly, so
# a security update flows through without pinning a version here.
SIMPLE = "https://pypi.org/simple/{}/"
# A pure-Python wheel: <name>-<version>-py3-none-any.whl. The name in the file
# uses underscores even when the distribution name uses dashes.
# A pure-Python wheel: <name>-<version>-py3-none-any.whl or the older
# <name>-<version>-py2.py3-none-any.whl. The name in the file uses underscores
# even when the distribution name uses dashes.
WHEEL_RE = re.compile(
    r"^([A-Za-z0-9_.]+)-([0-9][^-]*)-(?:py3|py2\.py3)-none-any\.whl$", re.I
)


def latest_url(name: str) -> tuple[str, str, str]:
    """The newest pure-Python wheel for a package.

    Returns the version, the download URL and the sha256 PyPI publishes for
    that exact file. The hash is not optional: the wheel is unpacked into the
    signed application bundle, so an unverified download would put whatever
    the network served straight into a notarized binary.
    """
    req = urllib.request.Request(SIMPLE.format(name), headers={"User-Agent": "iridium-sheets-build"})
    with urllib.request.urlopen(req, timeout=60) as r:
        html = r.read().decode("utf-8", "replace")
    best: tuple[str, str, str] | None = None
    for href in re.findall(r'href="([^"]+)"', html):
        if href.startswith("./"):
            href = href[2:]
        url = urllib.parse.urljoin(SIMPLE.format(name), href)
        # The href carries a #sha256=... fragment; the filename is before it.
        digest = ""
        if "#" in url:
            url, _, frag = url.partition("#")
            m = re.match(r"sha256=([0-9a-f]{64})\s*$", frag, re.I)
            if m:
                digest = m.group(1).lower()
        fn = url.rsplit("/", 1)[-1]
        m = WHEEL_RE.match(fn)
        if not m:
            continue
        version = m.group(2)
        # A prerelease (3.0.0.dev0) must not win over the latest stable.
        if not _is_stable(version):
            continue
        if best is None or _version_key(version) > _version_key(best[0]):
            best = (version, url, digest)
    if best is None:
        raise RuntimeError(f"no stable pure-Python wheel found for {name}")
    if not best[2]:
        raise RuntimeError(f"PyPI published no sha256 for {name} {best[0]}; refusing to fetch it unverified")
    return best


def _is_stable(version: str) -> bool:
    """True when a version is a final release, not an alpha, beta or dev."""
    return not re.search(r"(a|b|rc|dev|post|pre)\d*$", version, re.I)


def _version_key(version: str) -> tuple:
    """Order versions so 1.10 beats 1.9."""
    parts = []
    for p in re.split(r"[._-]", version):
        parts.append((0, int(p)) if p.isdigit() else (1, p))
    return tuple(parts)


def fetch_one(name: str, dest: Path) -> None:
    version, url, digest = latest_url(name)
    tmp = dest / f".{name}-{version}.whl"
    print(f"  {name} {version}", flush=True)
    with urllib.request.urlopen(url, timeout=300) as r, open(tmp, "wb") as f:
        shutil.copyfileobj(r, f)

    # Verify before unpacking. A wheel is a zip: unpacking it writes Python
    # that runs inside the app, so the bytes have to match what PyPI
    # published for that exact file.
    h = hashlib.sha256()
    with open(tmp, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    got = h.hexdigest()
    if got != digest:
        tmp.unlink(missing_ok=True)
        raise RuntimeError(
            f"{name} {version}: sha256 mismatch\n  expected {digest}\n  got      {got}"
        )

    with zipfile.ZipFile(tmp) as z:
        z.extractall(dest)
    tmp.unlink()


def main() -> int:
    if len(sys.argv) < 2:
        print(__doc__)
        return 1
    dest = Path(sys.argv[1])
    dest.mkdir(parents=True, exist_ok=True)

    for dist in PACKAGES:
        fetch_one(dist, dest)

    # Trim the discovery cache to the two APIs the app uses.
    docs = dest / "googleapiclient" / "discovery_cache" / "documents"
    if docs.exists():
        for f in docs.iterdir():
            if f.name not in DISCOVERY:
                f.unlink()
        print(f"  discovery cache trimmed to {len(DISCOVERY)} documents")

    # Byte-code caches are rebuilt on first run; they only add size.
    for cache in dest.rglob("__pycache__"):
        shutil.rmtree(cache, ignore_errors=True)

    # Prove the libraries import with only this directory on the path. A
    # missing dependency is otherwise found by the user, in the app, as
    # "No module named ...".
    check_imports(dest)
    print(f"  ready: {dest}")
    return 0


def check_imports(dest: Path) -> None:
    """Fail if the libraries do not import from `dest` alone."""
    import subprocess
    import sys as _sys

    script = (
        "import googleapiclient.discovery, google.oauth2.credentials, "
        "google_auth_oauthlib.flow, httplib2, requests_oauthlib"
    )
    env = dict(os.environ)
    env["PYTHONPATH"] = str(dest)
    proc = subprocess.run(
        [_sys.executable, "-c", script], capture_output=True, text=True, env=env, timeout=120
    )
    if proc.returncode != 0:
        raise RuntimeError(
            "the fetched libraries do not import: "
            + (proc.stderr.strip().splitlines() or ["unknown error"])[-1]
        )



if __name__ == "__main__":
    raise SystemExit(main())
