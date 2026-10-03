#!/usr/bin/env python3
"""Wait until the sparse index resolves the exact trusted published artifact."""

from __future__ import annotations

import argparse
import hashlib
import io
import json
import math
import re
import sys
import tarfile
import time
import urllib.error
import urllib.request


def fetch(url: str, timeout: float) -> bytes:
    request = urllib.request.Request(url, headers={"User-Agent": "everruns-ci"})
    deadline = time.monotonic() + timeout
    chunks = []
    size = 0
    with urllib.request.urlopen(request, timeout=timeout) as response:
        while True:
            if time.monotonic() >= deadline:
                raise TimeoutError("Registry response exceeded request deadline")
            chunk = response.read1(64 * 1024)
            if not chunk:
                return b"".join(chunks)
            size += len(chunk)
            if size > 64 * 1024 * 1024:
                raise ValueError("Registry response exceeds 64 MiB")
            chunks.append(chunk)


def index_path(name: str) -> str:
    name = name.lower()
    if len(name) < 3:
        return f"{len(name)}/{name}"
    if len(name) == 3:
        return f"3/{name[0]}/{name}"
    return f"{name[:2]}/{name[2:4]}/{name}"


def artifact_checksum(body: bytes, package: str, version: str, sha: str) -> str:
    # The publisher has already compared this tarball with its verified local
    # artifact. Derive the index's expected checksum independently of the index,
    # and keep the controller bound to that same clean release commit.
    with tarfile.open(fileobj=io.BytesIO(body), mode="r:gz") as archive:
        member = archive.getmember(f"{package}-{version}/.cargo_vcs_info.json")
        if not member.isfile() or member.size > 4096:
            raise ValueError("Invalid published VCS metadata")
        vcs = json.load(archive.extractfile(member))
    if not isinstance(vcs, dict) or not isinstance(vcs.get("git"), dict):
        raise ValueError("Invalid published VCS metadata")
    if vcs["git"].get("sha1") != sha or vcs["git"].get("dirty", False):
        raise ValueError("Published artifact does not match the clean release SHA")
    return hashlib.sha256(body).hexdigest()


def index_visible(body: bytes, package: str, version: str, checksum: str) -> bool:
    entries = [json.loads(line) for line in body.splitlines() if line.strip()]
    if not all(isinstance(entry, dict) for entry in entries):
        raise ValueError("Invalid sparse-index records")
    matches = [entry for entry in entries if entry.get("vers") == version]
    if not matches:
        return False
    if len(matches) != 1:
        raise ValueError(f"Duplicate sparse-index entries for {package} {version}")
    entry = matches[0]
    if entry.get("name") != package or entry.get("yanked") is not False:
        raise ValueError(f"Sparse-index entry for {package} {version} is invalid or yanked")
    if entry.get("cksum") != checksum:
        raise ValueError(f"Sparse-index checksum mismatch for {package} {version}")
    return True


def wait_for_index(package, version, sha, timeout=180, interval=2, *,
                   get=fetch, now=time.monotonic, sleep=time.sleep, report=print):
    start = now()
    deadline = start + timeout
    checksum = None
    attempts = 0
    last = "not checked"
    while now() < deadline:
        attempts += 1
        try:
            if checksum is None:
                body = get(f"https://static.crates.io/crates/{package}/{package}-{version}.crate",
                           min(10, deadline - now()))
                checksum = artifact_checksum(body, package, version, sha)
            if now() >= deadline:
                break
            body = get(f"https://index.crates.io/{index_path(package)}",
                       min(10, deadline - now()))
            if index_visible(body, package, version, checksum):
                elapsed = now() - start
                if now() >= deadline:
                    break
                report(f"Sparse index verified {package} {version}: checksum {checksum}; "
                       f"{elapsed:.2f}s, {attempts} attempt(s)")
                return elapsed
            last = "exact version absent from sparse index"
        except urllib.error.HTTPError as error:
            if error.code != 404 and error.code != 429 and error.code < 500:
                raise ValueError(f"Registry returned HTTP {error.code}") from error
            last = f"registry HTTP {error.code}"
        except (urllib.error.URLError, TimeoutError, ConnectionError) as error:
            last = f"transient registry transport failure ({type(error).__name__})"
        report(f"Waiting for {package} {version}: {last} (attempt {attempts})")
        sleep(max(0, min(interval, deadline - now())))
    raise TimeoutError(f"Sparse index did not verify {package} {version} within {timeout}s "
                       f"after {attempts} attempt(s): {last}")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--package", required=True)
    parser.add_argument("--version", required=True)
    parser.add_argument("--sha", required=True)
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--interval", type=float, default=2)
    args = parser.parse_args()
    if not re.fullmatch(r"[a-z0-9][a-z0-9_-]*", args.package):
        parser.error("invalid package name")
    if not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z]+(?:[.-][0-9A-Za-z]+)*)?", args.version):
        parser.error("invalid package version")
    if not re.fullmatch(r"[0-9a-f]{40}", args.sha):
        parser.error("invalid release SHA")
    if not all(math.isfinite(n) and n > 0 for n in (args.timeout, args.interval)):
        parser.error("timeout and interval must be finite positive numbers")
    try:
        wait_for_index(args.package, args.version, args.sha, args.timeout, args.interval)
    except (ValueError, TimeoutError, tarfile.TarError, KeyError) as error:
        print(f"::error::{error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
