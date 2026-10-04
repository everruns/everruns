#!/usr/bin/env bash
# Exercise scripts/lib/check-node-deps-lockfile.sh against scratch app trees.
#
# The regression this file exists for: apps/ui/node_modules held oxfmt 0.51.0
# while the lockfile resolved 0.70.0. `pnpm run format:check` therefore ran the
# old formatter, flagged two correctly-formatted files, and passed again once
# they were rewritten into the old formatter's style — so `just pre-push`
# reported "UI format" green on a change CI then rejected. The guard has to bite
# on exactly that shape: a tree whose installed version differs from the
# lockfile's resolved version, with no change to the lockfile's mtime.
#
# The cases use loose directories rather than git repos: the guard reads the
# lockfile and node_modules only, and never consults git.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
REPO_ROOT="$(pwd)"
GUARD="scripts/lib/check-node-deps-lockfile.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

fail_case() {
  echo "FAIL: $1" >&2
  failures=$((failures + 1))
}

# An app tree under the repo root, since the guard takes a repo-relative path.
# Returns the relative path.
scratch_app() {
  local name="$1"
  local rel=".tmp-node-deps-test/$name"
  local dir="$REPO_ROOT/$rel"
  rm -rf "$dir"
  mkdir -p "$dir"
  printf '%s\n' "$rel"
}

write_lockfile() {
  local dir="$1" pkg="$2" version="$3"
  cat >"$dir/pnpm-lock.yaml" <<EOF
lockfileVersion: '9.0'

settings:
  autoInstallPeers: true

importers:

  .:
    devDependencies:
      $pkg:
        specifier: ^$version
        version: $version

packages:

  $pkg@$version:
    resolution: {integrity: sha512-fake}
EOF
}

install_package() {
  local dir="$1" pkg="$2" version="$3"
  mkdir -p "$dir/node_modules/$pkg"
  cat >"$dir/node_modules/$pkg/package.json" <<EOF
{ "name": "$pkg", "version": "$version" }
EOF
}

cleanup_scratch() { rm -rf "$REPO_ROOT/.tmp-node-deps-test"; }
trap 'cleanup_scratch; rm -rf "$WORK"' EXIT

# 1. Installed version matches the lockfile -> pass.
rel="$(scratch_app matching)"
write_lockfile "$REPO_ROOT/$rel" oxfmt 0.70.0
install_package "$REPO_ROOT/$rel" oxfmt 0.70.0
if ! bash "$GUARD" "$rel" >/dev/null 2>&1; then
  fail_case "an in-sync tree must pass"
fi

# 2. The actual regression: installed version behind the lockfile, lockfile
#    untouched. This is what an mtime comparison cannot see.
rel="$(scratch_app drifted)"
write_lockfile "$REPO_ROOT/$rel" oxfmt 0.70.0
install_package "$REPO_ROOT/$rel" oxfmt 0.51.0
# Make node_modules strictly newer than the lockfile, which is the state that
# convinces an mtime heuristic the install is current.
touch "$REPO_ROOT/$rel/node_modules/oxfmt/package.json"
if output="$(bash "$GUARD" "$rel" 2>&1)"; then
  fail_case "a tree whose installed version lags the lockfile must fail"
else
  case "$output" in
    *"installed 0.51.0"*"0.70.0"*) ;;
    *) fail_case "drift message must name both versions, got: $output" ;;
  esac
fi

# 3. A direct dependency the lockfile resolves but nothing installed -> fail.
rel="$(scratch_app missing-package)"
write_lockfile "$REPO_ROOT/$rel" oxfmt 0.70.0
mkdir -p "$REPO_ROOT/$rel/node_modules"
if bash "$GUARD" "$rel" >/dev/null 2>&1; then
  fail_case "a lockfile dependency that is not installed must fail"
fi

# 4. No node_modules at all -> fail, rather than silently passing.
rel="$(scratch_app not-installed)"
write_lockfile "$REPO_ROOT/$rel" oxfmt 0.70.0
if bash "$GUARD" "$rel" >/dev/null 2>&1; then
  fail_case "a missing node_modules must fail"
fi

# 5. An app with no lockfile has nothing to be stale against -> pass.
rel="$(scratch_app no-lockfile)"
if ! bash "$GUARD" "$rel" >/dev/null 2>&1; then
  fail_case "an app without a lockfile must pass"
fi

# 6. A peer-dependency suffix is a resolution detail, not a version mismatch.
rel="$(scratch_app peer-suffix)"
mkdir -p "$REPO_ROOT/$rel"
cat >"$REPO_ROOT/$rel/pnpm-lock.yaml" <<'EOF'
lockfileVersion: '9.0'

importers:

  .:
    devDependencies:
      openapi-typescript:
        specifier: ^7.13.0
        version: 7.13.0(typescript@6.0.3)
EOF
install_package "$REPO_ROOT/$rel" openapi-typescript 7.13.0
if ! bash "$GUARD" "$rel" >/dev/null 2>&1; then
  fail_case "a peer-dependency suffix must not read as a mismatch"
fi

# 7. Package names repeat under `packages:` and `snapshots:` with versions that
#    are not the importer's. Only the importer block defines direct deps.
rel="$(scratch_app nested-sections)"
mkdir -p "$REPO_ROOT/$rel"
cat >"$REPO_ROOT/$rel/pnpm-lock.yaml" <<'EOF'
lockfileVersion: '9.0'

importers:

  .:
    devDependencies:
      oxfmt:
        specifier: ^0.70.0
        version: 0.70.0

packages:

  oxfmt@0.70.0:
    resolution: {integrity: sha512-fake}

snapshots:

  oxfmt@0.70.0:
    dependencies:
      some-transitive:
        version: 9.9.9
EOF
install_package "$REPO_ROOT/$rel" oxfmt 0.70.0
if ! bash "$GUARD" "$rel" >/dev/null 2>&1; then
  fail_case "only importer entries may count as direct dependencies"
fi

if [ "$failures" -ne 0 ]; then
  echo "node-deps lockfile guard: $failures case(s) failed" >&2
  exit 1
fi

echo "node-deps lockfile guard: all cases passed"
