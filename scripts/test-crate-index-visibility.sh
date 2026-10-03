#!/usr/bin/env bash
# Exercise the release controller's exact-artifact sparse-index gate offline.
set -euo pipefail
cd "$(dirname "${BASH_SOURCE[0]}")/.."
python3 - <<'PY'
import hashlib
import importlib.util
import io
import json
import pathlib
import tarfile
import unittest
import urllib.error

spec = importlib.util.spec_from_file_location("gate", "scripts/lib/wait-for-crate-index.py")
gate = importlib.util.module_from_spec(spec)
spec.loader.exec_module(gate)
SHA = "a" * 40
PKG = "everruns-core"
VER = "0.35.0"


def artifact(sha=SHA, dirty=False):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w:gz") as archive:
        body = json.dumps({"git": {"sha1": sha, "dirty": dirty}}).encode()
        member = tarfile.TarInfo(f"{PKG}-{VER}/.cargo_vcs_info.json")
        member.size = len(body)
        archive.addfile(member, io.BytesIO(body))
    return output.getvalue()


ARTIFACT = artifact()
CHECKSUM = hashlib.sha256(ARTIFACT).hexdigest()


def index(version=VER, checksum=CHECKSUM, yanked=False):
    return json.dumps({"name": PKG, "vers": version, "cksum": checksum, "yanked": yanked}).encode()


class Clock:
    seconds = 0

    def now(self):
        return self.seconds

    def sleep(self, seconds):
        self.seconds += seconds


class VisibilityTests(unittest.TestCase):
    def run_gate(self, responses, *, tarball=ARTIFACT, clock=None, timeout=5):
        clock = clock or Clock()
        replies = iter(responses)

        def get(url, request_timeout):
            self.assertGreater(request_timeout, 0)
            self.assertLessEqual(request_timeout, timeout)
            if url.startswith("https://static.crates.io/"):
                return tarball
            self.assertEqual(url, "https://index.crates.io/ev/er/everruns-core")
            reply = next(replies)
            if isinstance(reply, Exception):
                raise reply
            return reply

        logs = []
        elapsed = gate.wait_for_index(PKG, VER, SHA, timeout, 2, get=get,
                                      now=clock.now, sleep=clock.sleep, report=logs.append)
        return elapsed, logs

    def test_existing_exact_artifact_has_no_delay(self):
        elapsed, logs = self.run_gate([index()])
        self.assertEqual(elapsed, 0)
        self.assertIn("0.00s, 1 attempt", logs[-1])

    def test_old_version_cannot_open_gate(self):
        elapsed, logs = self.run_gate([index("0.34.2"), index()])
        self.assertEqual(elapsed, 2)
        self.assertIn("exact version absent", logs[0])

    def test_transport_and_404_are_retried(self):
        error = urllib.error.HTTPError("https://index.crates.io", 404, "missing", {}, None)
        elapsed, logs = self.run_gate([urllib.error.URLError("reset"), error, index()])
        self.assertEqual(elapsed, 4)
        self.assertIn("transport failure", logs[0])
        self.assertIn("HTTP 404", logs[1])

    def test_yanked_exact_version_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "yanked"):
            self.run_gate([index(yanked=True)])

    def test_wrong_checksum_fails_closed(self):
        with self.assertRaisesRegex(ValueError, "checksum mismatch"):
            self.run_gate([index(checksum="b" * 64)])

    def test_wrong_or_dirty_release_provenance_fails_closed(self):
        for body in [artifact(sha="b" * 40), artifact(dirty=True)]:
            with self.subTest(body=body), self.assertRaisesRegex(ValueError, "clean release SHA"):
                self.run_gate([], tarball=body)

    def test_duplicate_or_malformed_index_fails_closed(self):
        for body in [index() + b"\n" + index(), b"not json", b"null"]:
            with self.subTest(body=body), self.assertRaises(ValueError):
                self.run_gate([body])

    def test_missing_version_times_out_with_diagnostic(self):
        clock = Clock()
        with self.assertRaisesRegex(TimeoutError, "within 5s.*exact version absent"):
            self.run_gate([index("0.34.2")] * 3, clock=clock)
        self.assertEqual(clock.now(), 5)

    def test_slow_http_cannot_open_gate_after_deadline(self):
        clock = Clock()

        def get(url, timeout):
            clock.seconds += 3
            return ARTIFACT if "static.crates.io" in url else index()

        with self.assertRaisesRegex(TimeoutError, "within 5s"):
            gate.wait_for_index(PKG, VER, SHA, 5, 2, get=get, now=clock.now,
                                sleep=clock.sleep, report=lambda _: None)

    def test_unexpected_http_error_is_fatal(self):
        error = urllib.error.HTTPError("https://index.crates.io", 403, "forbidden", {}, None)
        with self.assertRaisesRegex(ValueError, "HTTP 403"):
            self.run_gate([error])

    def test_short_crate_index_paths(self):
        self.assertEqual([gate.index_path(n) for n in ["a", "ab", "abc", "ABCD"]],
                         ["1/a", "2/ab", "3/a/abc", "ab/cd/abcd"])

    def test_controller_uses_gate_instead_of_fixed_sleep(self):
        workflow = pathlib.Path(".github/workflows/crate-release.yml").read_text()
        self.assertIn("python3 scripts/lib/wait-for-crate-index.py", workflow)
        self.assertIn('--package "$PKG" --version "$VER" --sha "$SHA"', workflow)
        self.assertNotIn("sleep 25", workflow)


unittest.main(verbosity=2)
PY
