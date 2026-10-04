#!/usr/bin/env bash
# Check that a pnpm app's installed direct dependencies match the versions its
# pnpm-lock.yaml resolves, so local checks cannot run against a stale toolchain.
#
# Why this exists: `pnpm run format:check` and friends resolve their binaries
# from node_modules, and node_modules can lag the lockfile indefinitely. A tree
# installed before a formatter or linter bump keeps answering, with the old
# tool's rules. That makes a local gate worse than no gate: it reports green
# while disagreeing with CI, and "fixing" its complaints rewrites correct files
# into the old tool's style.
#
# An mtime comparison between pnpm-lock.yaml and node_modules/.modules.yaml
# cannot see this: the drift is in what was installed, not in when. So compare
# versions instead — the lockfile's resolved version for each direct dependency
# against the version in the installed package's own package.json.
#
# Used by: scripts/lib/pre-push.sh (gates the UI checks).
#
# Exits 0 when every direct dependency matches, 1 on mismatch or when the app's
# node_modules is absent. Exits 0 when the app directory itself has no
# pnpm-lock.yaml, since there is then nothing to be stale against.
#
# Usage: bash scripts/lib/check-node-deps-lockfile.sh apps/ui

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"

if [ "$#" -ne 1 ]; then
  echo "usage: $0 <app-dir-relative-to-repo-root>" >&2
  exit 1
fi

APP_DIR="$PROJECT_ROOT/$1"
LOCKFILE="$APP_DIR/pnpm-lock.yaml"

if [ ! -f "$LOCKFILE" ]; then
  exit 0
fi

if [ ! -d "$APP_DIR/node_modules" ]; then
  echo "error: $1/node_modules is missing; run: cd $1 && pnpm install"
  exit 1
fi

# Read the lockfile's importers section for this app's own direct dependencies
# and compare each against what is installed. Node does the reading because the
# installed version lives in JSON; the lockfile is scanned as text, since its
# importer entries are a fixed three-line shape (name, specifier, version) and
# the repo carries no YAML parser for shell.
MISMATCHES="$(
  APP_DIR="$APP_DIR" node -e '
const fs = require("fs");
const path = require("path");

const appDir = process.env.APP_DIR;
const lines = fs.readFileSync(path.join(appDir, "pnpm-lock.yaml"), "utf8").split("\n");

// Walk only the importers block, and only the entries for this app (".").
// Deeper keys (snapshots, packages) repeat package names and must not be read
// as direct dependencies.
let inImporters = false;
let inSelf = false;
let pending = null;
const wanted = new Map();

for (const line of lines) {
  if (/^[a-zA-Z]/.test(line)) {
    inImporters = line.startsWith("importers:");
    inSelf = false;
    continue;
  }
  if (!inImporters) continue;

  const importer = line.match(/^  (\S.*):\s*$/);
  if (importer) {
    inSelf = importer[1] === "." || importer[1] === "'\''.'\''";
    continue;
  }
  if (!inSelf) continue;

  // "    dependencies:" / "    devDependencies:" group headers.
  if (/^    \S+:\s*$/.test(line)) continue;

  const name = line.match(/^      (\S+):\s*$/);
  if (name) {
    pending = name[1].replace(/^'\''|'\''$/g, "");
    continue;
  }
  const version = line.match(/^        version:\s*(\S+)/);
  if (version && pending) {
    // Strip a peer-dependency suffix: 7.13.0(typescript@6.0.3) -> 7.13.0
    wanted.set(pending, version[1].replace(/\(.*$/, ""));
    pending = null;
  }
}

const problems = [];
for (const [name, expected] of wanted) {
  // A link: or file: resolution is a workspace path, not a version to compare.
  if (/^(link|file|workspace):/.test(expected)) continue;

  const manifest = path.join(appDir, "node_modules", name, "package.json");
  let installed;
  try {
    installed = JSON.parse(fs.readFileSync(manifest, "utf8")).version;
  } catch {
    problems.push(`${name}: not installed (lockfile resolves ${expected})`);
    continue;
  }
  if (installed !== expected) {
    problems.push(`${name}: installed ${installed}, lockfile resolves ${expected}`);
  }
}

process.stdout.write(problems.join("\n"));
'
)"

if [ -n "$MISMATCHES" ]; then
  echo "error: $1/node_modules does not match $1/pnpm-lock.yaml"
  printf '%s\n' "$MISMATCHES" | sed 's/^/  /'
  echo "  run: cd $1 && pnpm install"
  exit 1
fi

exit 0
