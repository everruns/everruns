#!/usr/bin/env bash
# Verify specs/README.md indexes every top-level specs/*.md file and does not
# reference missing top-level spec files.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SPECS_DIR="$PROJECT_ROOT/specs"
INDEX="$SPECS_DIR/README.md"

if [ ! -f "$INDEX" ]; then
  echo "error: missing specs index: specs/README.md"
  exit 1
fi

failures=0

while IFS= read -r spec_file; do
  rel_path="${spec_file#$PROJECT_ROOT/}"
  if ! grep -Fq "\`$rel_path\`" "$INDEX"; then
    echo "missing from specs/README.md: $rel_path"
    failures=$((failures + 1))
  fi
done < <(find "$SPECS_DIR" -maxdepth 1 -type f -name '*.md' ! -name 'README.md' | LC_ALL=C sort)

while IFS= read -r indexed_path; do
  if [ ! -f "$PROJECT_ROOT/$indexed_path" ]; then
    echo "indexed spec does not exist: $indexed_path"
    failures=$((failures + 1))
  fi
done < <(grep -Eo 'specs/[A-Za-z0-9._-]+\.md' "$INDEX" | LC_ALL=C sort -u)

if [ "$failures" -ne 0 ]; then
  echo "specs index coverage failed with $failures issue(s)"
  exit 1
fi

count="$(find "$SPECS_DIR" -maxdepth 1 -type f -name '*.md' ! -name 'README.md' | wc -l | tr -d ' ')"
echo "specs/README.md covers $count top-level specs"
