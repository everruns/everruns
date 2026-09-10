#!/usr/bin/env bash
# Exercise the real guard against isolated tracked crates and workflow commands.
set -euo pipefail
cd "$(dirname "$0")/.."
python3 - <<'PY'
from pathlib import Path
import shutil
import subprocess
import tempfile

with tempfile.TemporaryDirectory(prefix="everruns-enumeration-") as directory:
    root = Path(directory)
    (root / "scripts/lib").mkdir(parents=True)
    guard = root / "scripts/lib/check-test-enumeration.sh"
    shutil.copyfile("scripts/lib/check-test-enumeration.sh", guard)
    workflows = root / ".github/workflows"
    workflows.mkdir(parents=True)
    # Keep a top-level crate so the old depth-limited guard reports false success.
    anchor = root / "crates/anchor"
    (anchor / "tests").mkdir(parents=True)
    (anchor / "Cargo.toml").write_text('[package]\nname = "anchor"\n')
    (anchor / "tests/smoke.rs").write_text("")
    for package in ("first", "second", "library-only"):
        crate = root / "crates/drivers" / package
        (crate / "src").mkdir(parents=True)
        (crate / "Cargo.toml").write_text(f'[package]\nname = "{package}"\n')
        (crate / "src/lib.rs").write_text("")
        if package != "library-only":
            (crate / "tests").mkdir()
            (crate / "tests/chat_wire.rs").write_text("")
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    subprocess.run(["git", "-C", str(root), "add", "crates"], check=True)

    def check(commands, expected_errors):
        (workflows / "ci.yml").write_text("steps:\n  - run: |\n    cargo test -p anchor\n" + commands)
        result = subprocess.run(["bash", str(guard)], text=True, capture_output=True)
        assert result.returncode == (1 if expected_errors else 0), result.stdout + result.stderr
        for error in expected_errors:
            assert error in result.stdout, result.stdout

    # A comment and a library-only crate must not evade inventory discovery.
    check("    cargo test -p first -p second\n    # cargo test -p library-only\n",
          ["library-only: driver library has no unit-test invocation"])
    # Identically named targets belong to their package, not the global YAML.
    check("    cargo test -p first --lib\n    cargo test -p second --test chat_wire\n"
          "    cargo test -p second -p library-only --lib\n",
          ["first: crates/drivers/first/tests/chat_wire.rs"])
    # Continuations can carry both additional packages and named targets.
    check("    cargo test -p first \\\n      -p second --lib\n"
          "    cargo test -p first -p second \\\n      --test chat_wire\n"
          "    cargo test -p library-only --lib\n", [])
    check("    cargo test -p first -p second\n    cargo test -p library-only --doc\n",
          ["library-only: driver library has no unit-test invocation"])
    check("    cargo test -p first -p second\n"
          "    cargo test -p library-only --lib -- --ignored\n",
          ["library-only: driver library has no unit-test invocation"])
    # Complete crate runs cover libraries and integration targets together.
    check("    cargo test -p first -p second -p library-only --all-features\n", [])
print("Test enumeration: six isolated workflow scenarios passed.")
PY
