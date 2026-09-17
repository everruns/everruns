#!/usr/bin/env bash
# Guard the Crate Release tagging contract: release tags are reused, never
# moved, and a tag left behind by a failed attempt stops the run with an
# actionable message instead of dispatching a publish that cannot be trusted.
#
# The snippet under test is extracted from .github/workflows/crate-release.yml
# rather than restated here, so this exercises the shipped logic and cannot
# drift into testing a stale copy of it.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"
WORKFLOW="$PROJECT_ROOT/.github/workflows/crate-release.yml"

failures=()
require() { [ "$1" = "0" ] || failures+=("$2"); }

# --- Static contract: the tag is an anchor, so nothing may move it. ---------
if grep -qE 'git tag +-f|git tag +--force' "$WORKFLOW"; then
  failures+=("crate-release must never force-create a release tag")
fi
if grep -qE 'push .*--force.*refs/tags|push .*\+refs/tags' "$WORKFLOW"; then
  failures+=("crate-release must never force-push a release tag")
fi
grep -q 'A fix to the publish workflow' "$WORKFLOW" \
  || failures+=("crate-release must re-trigger on .github/workflows/publish-crates.yml")
grep -q "'.github/workflows/publish-crates.yml'" "$WORKFLOW" \
  || failures+=("crate-release paths must include publish-crates.yml")

# --- Behavioural: extract the real tag-resolution snippet and run it. -------
SNIPPET="$(awk '/^ *EXISTING=""$/{f=1} f&&/^ *# gh workflow run does not/{exit} f' \
  "$WORKFLOW" | sed -e 's/^            //')"
grep -q 'EXISTING=' <<<"$SNIPPET" || failures+=("could not extract the tag-resolution snippet")

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
(
  cd "$WORK"
  git init -q .
  git config user.email t@example.com
  git config user.name t
  git commit -q --allow-empty -m one
  OLD="$(git rev-parse HEAD)"
  git commit -q --allow-empty -m two
  NEW="$(git rev-parse HEAD)"
  git tag "crate/pkg/v1.0.0" "$OLD"
  printf '%s\n%s\n' "$OLD" "$NEW" > shas
) || failures+=("fixture repo setup failed")

OLD="$(sed -n 1p "$WORK/shas")"
NEW="$(sed -n 2p "$WORK/shas")"

run_snippet() { # $1 = SHA this release is publishing from
  (
    cd "$WORK"
    set +e
    PKG=pkg TAG="crate/pkg/v1.0.0" SHA="$1" GITHUB_REPOSITORY=o/r \
      bash -c "set -euo pipefail; $SNIPPET" 2>&1
    echo "EXIT=$?"
  )
}

mismatch="$(run_snippet "$NEW")"
grep -q "EXIT=1" <<<"$mismatch" \
  || failures+=("a tag pointing at another commit must fail the run")
grep -q "::error::Tag crate/pkg/v1.0.0 exists at $OLD" <<<"$mismatch" \
  || failures+=("the stale-tag error must name the tag and the commit it resolves to")
grep -q "DELETE repos/o/r/git/refs/tags/crate/pkg/v1.0.0" <<<"$mismatch" \
  || failures+=("the stale-tag error must name the exact deletion command")

match="$(run_snippet "$OLD")"
grep -q "EXIT=0" <<<"$match" \
  || failures+=("a tag already at this release's commit must be reused, not an error")
grep -q "reusing" <<<"$match" \
  || failures+=("reusing an existing correct tag must say so")

# The tag must still point where it did: the snippet may not move it.
AFTER="$(cd "$WORK" && git rev-list -n 1 'crate/pkg/v1.0.0^{commit}')"
[ "$AFTER" = "$OLD" ] || failures+=("the tag was moved; release tags are immutable anchors")

if [ "${#failures[@]}" -gt 0 ]; then
  printf 'FAIL: %s\n' "${failures[@]}" >&2
  exit 1
fi
echo "crate release tagging: stale tags fail closed with a remedy; tags are never moved"
