#!/usr/bin/env bash
# Exercise scripts/lib/check-file-size.sh against a scratch repository.
#
# The guard has to bite in both directions: reject a change that grows an
# oversized file or admits a new one, and refuse to let a file that dropped
# under the threshold stay on the debt list.
#
# The regression this file exists for is EVE-1105. Two PRs cut from the same
# base, neither growing the file, used to be able to turn `main` red once both
# landed, because the guard compared the file against a committed integer that
# either PR could move without conflicting. The last case here replays exactly
# that sequence and asserts `main` stays green.
#
# The guard runs against `git ls-files` and `git merge-base`, so every case is a
# real throwaway git repository rather than a loose directory.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
GUARD="$(pwd)/scripts/lib/check-file-size.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

# A scratch repo carrying a copy of the guard, on a `main` branch so the default
# base ref resolves the same way it does in the real repository.
scratch_repo() {
  local dir="$WORK/repo"
  rm -rf "$dir"
  mkdir -p "$dir/crates/demo/src" "$dir/scripts/lib"
  cp "$GUARD" "$dir/scripts/lib/check-file-size.sh"
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

# A Rust file of exactly $2 lines.
write_lines() {
  local path="$1" count="$2" i
  : > "$path"
  for ((i = 1; i <= count; i++)); do
    printf 'const L%d: u32 = %d;\n' "$i" "$i" >> "$path"
  done
}

# Run the guard with an explicit base, since a scratch repo has no remote.
expect() {
  local name="$1" expected="$2" dir="$3" base="$4" needle="${5:-}"
  local output status
  set +e
  output="$(cd "$dir" && FILE_SIZE_BASE_REF="$base" bash scripts/lib/check-file-size.sh 2>&1)"
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

# --- an unchanged oversized file passes -------------------------------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1600
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
expect "an unchanged oversized file passes" pass "$REPO" HEAD "none grew"

# --- an allowlisted file may not grow ---------------------------------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1600
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_lines "$REPO/crates/demo/src/lib.rs" 1650
commit_all "$REPO"
expect "growth is rejected" fail "$REPO" "$BASE" "grew to 1650 lines"

# --- shrinking is free, and needs no allowlist edit -------------------------
#
# The old guard demanded the committed number be ratcheted down here. That edit
# is what two PRs used to collide on.

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1600
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_lines "$REPO/crates/demo/src/lib.rs" 1550
commit_all "$REPO"
expect "shrinking needs no allowlist edit" pass "$REPO" "$BASE" "none grew"

# --- a brand new file may not cross the threshold ---------------------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 100
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_lines "$REPO/crates/demo/src/new.rs" 1700
commit_all "$REPO"
expect "a new oversized file is rejected" fail "$REPO" "$BASE" "do not add it to"

# --- allowlisting a brand new oversized file does not buy it in -------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 100
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_lines "$REPO/crates/demo/src/new.rs" 1700
echo "crates/demo/src/new.rs" >> "$REPO/scripts/lib/file-size-allowlist.txt"
commit_all "$REPO"
expect "hand-allowlisting a new file is rejected" fail "$REPO" "$BASE" "new on this branch"

# --- a renamed allowlisted file is measured against its old path ------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1600
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
git -C "$REPO" mv crates/demo/src/lib.rs crates/demo/src/moved.rs
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
expect "a pure rename passes" pass "$REPO" "$BASE" "none grew"

# --- a file that dropped under the threshold must bank the win --------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1600
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
write_lines "$REPO/crates/demo/src/lib.rs" 1400
commit_all "$REPO"
expect "a shrunk file must bank the win" fail "$REPO" "$BASE" "remove its entry"

# --- a deleted file leaves a stale entry ------------------------------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1600
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"
git -C "$REPO" rm -q crates/demo/src/lib.rs
commit_all "$REPO"
expect "a stale entry is rejected" fail "$REPO" "$BASE" "stale entry"

# --- an unresolvable base ref is an error, never a silent skip --------------

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1600
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
expect "an unresolvable base is an error" fail "$REPO" "refs/heads/nope" "Could not resolve"

# --- EVE-1105: two non-growing PRs must not turn main red -------------------
#
# Both branches shrink the same file by different amounts and touch nothing
# else. Under the old guard the first to land re-baselined the committed
# number, the second landed a longer file against it, and `main` went red on a
# file neither PR grew — with no git conflict anywhere, because the two commits
# touch different files.

REPO="$(scratch_repo)"
write_lines "$REPO/crates/demo/src/lib.rs" 1827
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
BASE="$(git -C "$REPO" rev-parse HEAD)"

# The branch that shrinks further lands first, and regenerates the allowlist on
# the way — the step that used to write the number the second branch then got
# measured against.
write_lines "$REPO/crates/demo/src/lib.rs" 1767
commit_all "$REPO"
expect "the tighter branch passes on its own" pass "$REPO" "$BASE" "none grew"
(cd "$REPO" && bash scripts/lib/check-file-size.sh --update >/dev/null)
commit_all "$REPO"
TIGHTENED="$(git -C "$REPO" rev-parse HEAD)"

# The second branch, cut from the same base, shrinks less. Against its own base
# it is a shrink and passes, which is what let it through review.
git -C "$REPO" checkout -q -b shrinks-less "$BASE"
write_lines "$REPO/crates/demo/src/lib.rs" 1825
commit_all "$REPO"
expect "the looser branch passes against its own base" pass "$REPO" "$BASE" "none grew"

# CI checks out the merge of the branch and current main, so the comparison is
# "what will main hold after this lands" against "what main holds now". That is
# where the collision surfaces — on the second PR, before it merges.
git -C "$REPO" checkout -q -b pr-merge "$TIGHTENED"
git -C "$REPO" merge -q --no-commit --no-ff shrinks-less >/dev/null 2>&1 || true
write_lines "$REPO/crates/demo/src/lib.rs" 1825
commit_all "$REPO"
expect "the collision is caught on the second PR" fail "$REPO" "$TIGHTENED" "grew to 1825 lines"

# And the state main ends up in once it lands anyway is not itself a failure:
# main is its own base, so nothing measures it against a number it never had.
git -C "$REPO" checkout -q main
write_lines "$REPO/crates/demo/src/lib.rs" 1825
commit_all "$REPO"
expect "main stays green after both land" pass "$REPO" HEAD "none grew"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "file size guard behaves in both directions"
