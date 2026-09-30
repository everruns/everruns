#!/usr/bin/env bash
# Guard write-scoped release workflows against expression injection and tag refs.

set -euo pipefail

python3 - <<'PY'
from pathlib import Path
import re

workflows = [
    Path(".github/workflows/cli-binaries.yml"),
    Path(".github/workflows/docker-publish.yml"),
    Path(".github/workflows/server-worker-binaries.yml"),
    Path(".github/workflows/publish-crates.yml"),
    Path(".github/workflows/release.yml"),
    Path(".github/workflows/crate-release.yml"),
    Path(".github/workflows/crate-yank.yml"),
]
untrusted = re.compile(r"\$\{\{[^\n]*(?:inputs\.|client_payload\.tag|ref_name)")
sha_ref = re.compile(r"uses:\s+([^./\s][^@\s]*)@([0-9a-f]{40})(?:\s+#\s+\S+)?$")
errors = []

for path in workflows:
    lines = path.read_text().splitlines()
    run_indent = None
    for number, line in enumerate(lines, 1):
        stripped = line.lstrip()
        indent = len(line) - len(stripped)
        if run_indent is not None and stripped and indent <= run_indent:
            run_indent = None
        if stripped.startswith("run: |"):
            run_indent = indent
            continue
        if run_indent is not None and untrusted.search(line):
            errors.append(f"{path}:{number}: untrusted release expression inside run block")
        if "uses:" in stripped and not re.search(r"uses:\s+\./", stripped):
            if not sha_ref.search(stripped):
                errors.append(f"{path}:{number}: external action is not pinned to a 40-char SHA")

if errors:
    raise SystemExit("\n".join(errors))
print("write-scoped release workflows use env-bound inputs and SHA-pinned actions")
PY

# GitHub Actions string functions compare without regard to case. Release
# authorization must instead preserve the case-sensitive commit convention.
release_workflow=$(<.github/workflows/release.yml)
if [[ "$release_workflow" == *"startsWith(github.event.head_commit.message"* ]]; then
  echo "release gate must not use case-insensitive GitHub Actions string functions" >&2
  exit 1
fi
if [[ "$release_workflow" != *'SUBJECT=$(git log -1 --format=%s HEAD)'* ]] ||
   [[ "$release_workflow" != *'[[ "$SUBJECT" == chore\(release\):\ prepare\ v* ]]'* ]]; then
  echo "release gate must check the checked-out subject with a case-sensitive Bash pattern" >&2
  exit 1
fi

is_release_subject() {
  local subject=$1
  [[ "$subject" == chore\(release\):\ prepare\ v* ]]
}

is_release_subject 'chore(release): prepare v1.2.3 (#123)'
for subject in \
  'Chore(release): prepare v1.2.3 (#123)' \
  'chore(Release): prepare v1.2.3 (#123)' \
  'chore(release): Prepare v1.2.3 (#123)'; do
  if is_release_subject "$subject"; then
    echo "mixed-case release subject was accepted: $subject" >&2
    exit 1
  fi
done

echo "release subject authorization is case-sensitive"
