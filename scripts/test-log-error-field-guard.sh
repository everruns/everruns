#!/usr/bin/env bash
# Exercise scripts/lib/check-log-error-field.sh against a scratch repository.
#
# A ratchet is only worth its maintenance if it bites in both directions: it has
# to reject a new interpolated site, and it has to refuse to let a file that
# reached zero stay on the debt list. The second half is what stops the debt
# quietly re-accumulating behind a stale baseline.
#
# The regression the last case exists for is EVE-1107, the same shape as
# EVE-1105. Two PRs cut from one base, neither adding a site, used to be able to
# turn `main` red once both landed, because the guard compared each file against
# a count committed next to its path that either PR could move without
# conflicting.
#
# The guard runs against `git ls-files` and `git merge-base`, so every case here
# is a real throwaway git repository rather than a loose directory.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
GUARD="$(pwd)/scripts/lib/check-log-error-field.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

# A scratch repo on a `main` branch, so the default base ref resolves the same
# way it does in the real repository.
scratch_repo() {
  local dir="$WORK/repo"
  rm -rf "$dir"
  mkdir -p "$dir/crates/demo/src" "$dir/scripts/lib"
  cp "$GUARD" "$dir/scripts/lib/check-log-error-field.sh"
  git -C "$dir" init -q -b main
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name Test
  printf '%s\n' "$dir"
}

commit_all() {
  git -C "$1" add -A
  # A step that turns out to change nothing is a result, not an error: the
  # regenerate-the-allowlist step is a no-op under the current guard and the
  # edit the old one demanded under its predecessor.
  git -C "$1" diff --cached --quiet && return 0
  git -C "$1" -c commit.gpgsign=false commit -qm fixture
}

# $2 interpolated sites in $1, plus one structured site that is never one.
write_sites() {
  local path="$1" count="$2" i
  {
    printf 'fn structured(e: String) { tracing::error!(error = %%e, "Structured"); }\n'
    for ((i = 1; i <= count; i++)); do
      printf 'fn interpolated%d(e: String) { tracing::error!("Step %d failed: {}", e); }\n' "$i" "$i"
    done
  } > "$path"
}

# Run the guard with an explicit base, since a scratch repo has no remote.
expect() {
  local name="$1" expected="$2" dir="$3" base="$4" needle="${5:-}"
  local output status
  set +e
  output="$(cd "$dir" && LOG_ERROR_FIELD_BASE_REF="$base" bash scripts/lib/check-log-error-field.sh 2>&1)"
  status=$?
  set -e
  if [ "$expected" = "pass" ] && [ "$status" -ne 0 ]; then
    echo "FAIL $name: expected the guard to pass, got:"
    echo "$output" | sed 's/^/    /'
    failures=$((failures + 1))
    return
  fi
  if [ "$expected" = "fail" ] && [ "$status" -eq 0 ]; then
    echo "FAIL $name: expected the guard to fail, it passed:"
    echo "$output" | sed 's/^/    /'
    failures=$((failures + 1))
    return
  fi
  if [ -n "$needle" ] && ! grep -qF "$needle" <<<"$output"; then
    echo "FAIL $name: expected output to mention '$needle', got:"
    echo "$output" | sed 's/^/    /'
    failures=$((failures + 1))
    return
  fi
  echo "ok   $name"
}

# --- an unchanged baselined file passes -------------------------------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 2
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
expect "an unchanged baselined file passes" pass "$REPO" HEAD "none grew"

# --- the structured form is not a site at all -------------------------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 0
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
expect "structured form is clean" pass "$REPO" HEAD "0 sites remaining"

# --- an allowlisted file may not gain a site --------------------------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 1
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_sites "$REPO/crates/demo/src/lib.rs" 2
commit_all "$REPO"
expect "growth is rejected" fail "$REPO" "$BASE" "grew to 2 interpolated sites"

# --- converting a site needs no allowlist edit ------------------------------
#
# The old guard demanded the committed count be ratcheted down here. That edit
# is what two PRs used to collide on.

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 3
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_sites "$REPO/crates/demo/src/lib.rs" 1
commit_all "$REPO"
expect "converting needs no allowlist edit" pass "$REPO" "$BASE" "none grew"

# --- a brand new file may not introduce one ---------------------------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 0
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_sites "$REPO/crates/demo/src/new.rs" 1
commit_all "$REPO"
expect "a new file is rejected" fail "$REPO" "$BASE" "do not add it to"

# --- allowlisting a brand new file does not buy it in -----------------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 0
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_sites "$REPO/crates/demo/src/new.rs" 1
echo "crates/demo/src/new.rs" >> "$REPO/scripts/lib/log-error-field-allowlist.txt"
commit_all "$REPO"
expect "hand-allowlisting a new file is rejected" fail "$REPO" "$BASE" "new on this branch"

# --- a renamed file is measured against its old path ------------------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 2
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
git -C "$REPO" mv crates/demo/src/lib.rs crates/demo/src/moved.rs
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
expect "a pure rename passes" pass "$REPO" "$BASE" "none grew"

# --- a file that reached zero must be removed from the list -----------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 1
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_sites "$REPO/crates/demo/src/lib.rs" 0
commit_all "$REPO"
expect "a cleared file must bank the win" fail "$REPO" "$BASE" "remove its entry"

# --- a deleted file leaves a stale entry ------------------------------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 1
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
git -C "$REPO" rm -q crates/demo/src/lib.rs
commit_all "$REPO"
expect "a stale entry is rejected" fail "$REPO" "$BASE" "stale entry"

# --- an unresolvable base ref is an error, never a silent skip --------------

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 1
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
expect "an unresolvable base is an error" fail "$REPO" "refs/heads/nope" "Could not resolve"

# --- EVE-1107: two non-growing PRs must not turn main red -------------------
#
# Both branches convert sites in the same file by different amounts and touch
# nothing else. Under the old guard the first to land re-baselined the committed
# count, the second landed a higher count against it, and `main` went red on a
# file neither PR added a site to — with no git conflict anywhere, because the
# two commits touch different files.

REPO="$(scratch_repo)"
write_sites "$REPO/crates/demo/src/lib.rs" 5
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"

# The branch that converts more lands first, regenerating the allowlist on the
# way — the step that used to write the count the second branch got measured
# against.
write_sites "$REPO/crates/demo/src/lib.rs" 1
commit_all "$REPO"
expect "the tighter branch passes on its own" pass "$REPO" "$BASE" "none grew"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
commit_all "$REPO"
CONVERTED="$(git -C "$REPO" rev-parse HEAD)"

# The second branch, cut from the same base, converts less. Against its own base
# that is a reduction and passes, which is what let it through review.
git -C "$REPO" checkout -q -b converts-less "$BASE"
write_sites "$REPO/crates/demo/src/lib.rs" 3
commit_all "$REPO"
expect "the looser branch passes against its own base" pass "$REPO" "$BASE" "none grew"

# CI checks out the merge of the branch and current main, so the comparison is
# "what will main hold after this lands" against "what main holds now". That is
# where the collision surfaces — on the second PR, before it merges.
git -C "$REPO" checkout -q -b pr-merge "$CONVERTED"
git -C "$REPO" merge -q --no-commit --no-ff converts-less >/dev/null 2>&1 || true
write_sites "$REPO/crates/demo/src/lib.rs" 3
commit_all "$REPO"
expect "the collision is caught on the second PR" fail "$REPO" "$CONVERTED" "grew to 3 interpolated sites"

# And the state main ends up in once it lands anyway is not itself a failure:
# main is its own base, so nothing measures it against a count it never had.
git -C "$REPO" checkout -q main
write_sites "$REPO/crates/demo/src/lib.rs" 3
commit_all "$REPO"
expect "main stays green after both land" pass "$REPO" HEAD "none grew"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "log error-field guard behaves in both directions"
