#!/usr/bin/env bash
# `cargo tree` for the dependency guards, with its stderr kept.
#
# Why this exists: the guards ran `cargo tree ... 2>/dev/null` under
# `set -euo pipefail`. When cargo failed for a reason that had nothing to do
# with the repository — a registry hiccup, lock contention, a cold CARGO_HOME —
# stderr went to /dev/null and `set -e` exited 101, so the job printed
# `Process completed with exit code 101` and nothing else.
#
# That made an infrastructure failure indistinguishable from a real
# architectural violation, which is the one distinction these guards exist to
# draw. Four opaque CI failures on 2026-09-22 were traced back to it.
#
# `guard_cargo_tree` keeps stderr, and on failure says plainly that the guard
# could not run, prints what cargo actually said, and exits 2 — a code no guard
# uses for a violation, so "we could not check" and "we checked and it is
# broken" stay different answers.
#
# Sourced by: the isolation and dependency guards in this directory.

# Exit code for "the guard could not run", distinct from 1 (a violation).
GUARD_INFRA_EXIT=2

guard_cargo_tree() {
  local stderr_file output status
  stderr_file="$(mktemp)"

  # `set -e` must not fire here: the whole point is to report the failure
  # ourselves rather than die silently.
  set +e
  output="$(cargo tree "$@" 2>"$stderr_file")"
  status=$?
  set -e

  if [ "$status" -ne 0 ]; then
    {
      echo "Guard could not run: 'cargo tree $*' exited $status."
      echo "This is not an architectural violation — the check never completed."
      echo "cargo said:"
      sed 's/^/    /' "$stderr_file"
    } >&2
    rm -f "$stderr_file"
    exit "$GUARD_INFRA_EXIT"
  fi

  rm -f "$stderr_file"
  printf '%s\n' "$output"
}
