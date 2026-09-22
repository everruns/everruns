#!/usr/bin/env bash
# Exercise scripts/lib/check-log-error-field.sh against a scratch repository.
#
# A ratchet is only worth its maintenance if it bites in both directions: it has
# to reject a new interpolated site, and it has to refuse to let a file that
# reached zero stay on the debt list. The second half is what stops the debt
# quietly re-accumulating behind a stale baseline.
#
# The guard runs against `git ls-files`, so every case here is a real throwaway
# git repository rather than a loose directory.

set -euo pipefail

cd "$(dirname "${BASH_SOURCE[0]}")/.."
GUARD="$(pwd)/scripts/lib/check-log-error-field.sh"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

failures=0

# A scratch repo whose crates/ contains exactly the given file bodies.
scratch_repo() {
  local dir="$WORK/repo"
  rm -rf "$dir"
  mkdir -p "$dir/crates/demo/src" "$dir/scripts/lib"
  cp "$GUARD" "$dir/scripts/lib/check-log-error-field.sh"
  git -C "$dir" init -q
  git -C "$dir" config user.email test@example.com
  git -C "$dir" config user.name Test
  printf '%s\n' "$dir"
}

commit_all() {
  git -C "$1" add -A
  git -C "$1" -c commit.gpgsign=false commit -qm fixture
}

expect() {
  local name="$1" expected="$2" dir="$3" needle="${4:-}"
  local output status
  set +e
  output="$(cd "$dir" && bash scripts/lib/check-log-error-field.sh 2>&1)"
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

# --- a baselined file at its recorded count passes --------------------------

REPO="$(scratch_repo)"
cat > "$REPO/crates/demo/src/lib.rs" <<'RS'
fn a(e: String) { tracing::error!("Alpha failed: {}", e); }
fn b(e: String) { tracing::warn!("Beta failed: {}", e); }
RS
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
expect "baseline passes" pass "$REPO" "none grew"

# --- the structured form is not a site at all -------------------------------

REPO="$(scratch_repo)"
cat > "$REPO/crates/demo/src/lib.rs" <<'RS'
fn a(e: String) { tracing::error!(error = %e, "Alpha failed"); }
RS
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
expect "structured form is clean" pass "$REPO" "0 sites remaining"

# --- an allowlisted file may not gain a site --------------------------------

REPO="$(scratch_repo)"
cat > "$REPO/crates/demo/src/lib.rs" <<'RS'
fn a(e: String) { tracing::error!("Alpha failed: {}", e); }
RS
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
cat >> "$REPO/crates/demo/src/lib.rs" <<'RS'
fn b(e: String) { tracing::error!("Beta failed: {}", e); }
RS
commit_all "$REPO"
expect "growth is rejected" fail "$REPO" "grew to 2 interpolated sites"

# --- a brand new file may not introduce one ---------------------------------

REPO="$(scratch_repo)"
cat > "$REPO/crates/demo/src/lib.rs" <<'RS'
fn a(e: String) { tracing::error!(error = %e, "Alpha failed"); }
RS
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
cat > "$REPO/crates/demo/src/new.rs" <<'RS'
fn c(e: String) { tracing::error!("Gamma failed: {}", e); }
RS
commit_all "$REPO"
expect "a new file is rejected" fail "$REPO" "do not add it to"

# --- a file that reached zero must be removed from the list -----------------
#
# Without this the debt list keeps a baseline the file no longer needs, and the
# same sites can come back later for free.

REPO="$(scratch_repo)"
cat > "$REPO/crates/demo/src/lib.rs" <<'RS'
fn a(e: String) { tracing::error!("Alpha failed: {}", e); }
RS
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
cat > "$REPO/crates/demo/src/lib.rs" <<'RS'
fn a(e: String) { tracing::error!(error = %e, "Alpha failed"); }
RS
commit_all "$REPO"
expect "a cleared file must bank the win" fail "$REPO" "remove its entry"

# --- a deleted file leaves a stale entry ------------------------------------

REPO="$(scratch_repo)"
cat > "$REPO/crates/demo/src/lib.rs" <<'RS'
fn a(e: String) { tracing::error!("Alpha failed: {}", e); }
RS
commit_all "$REPO"
(cd "$REPO" && bash scripts/lib/check-log-error-field.sh --update >/dev/null)
git -C "$REPO" rm -q crates/demo/src/lib.rs
commit_all "$REPO"
expect "a stale entry is rejected" fail "$REPO" "stale entry"

if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) failed"
  exit 1
fi
echo "log error-field guard behaves in both directions"
