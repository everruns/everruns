#!/usr/bin/env bash
# Verify the canonical knowledge bundle's OKF v0.2 structure and links.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
CHECKER="$PROJECT_ROOT/scripts/check_okf.py"
TMP_DIR="$(mktemp -d)"
trap 'rm -rf "$TMP_DIR"' EXIT

write_bundle() {
  local bundle="$1"
  local first_title="$2"
  local second_id="$3"
  local second_title="$4"

  mkdir -p "$bundle/test-cases/ui/widgets" "$bundle/test-cases/ui/gadgets"
  cat > "$bundle/index.md" <<'EOF'
---
okf_version: "0.2"
---
# Knowledge

- [Test cases](test-cases/)
EOF
  cat > "$bundle/test-cases/index.md" <<'EOF'
# Test cases

- [UI](ui/)
EOF
  cat > "$bundle/test-cases/ui/index.md" <<'EOF'
# UI

- [Widgets](widgets/)
- [Gadgets](gadgets/)
EOF
  cat > "$bundle/test-cases/ui/widgets/index.md" <<EOF
# Widgets

- [First case](TC001_first.md)
- [Second case](${second_id}_second.md)
EOF
  cat > "$bundle/test-cases/ui/widgets/TC001_first.md" <<EOF
---
type: Test Case
title: "$first_title"
description: "First fixture case."
---
# $first_title
EOF
  cat > "$bundle/test-cases/ui/widgets/${second_id}_second.md" <<EOF
---
type: Test Case
title: "$second_title"
description: "Second fixture case."
---
# $second_title
EOF
  cat > "$bundle/test-cases/ui/gadgets/index.md" <<'EOF'
# Gadgets

- [Create a gadget](TC001_create.md)
EOF
  cat > "$bundle/test-cases/ui/gadgets/TC001_create.md" <<'EOF'
---
type: Test Case
title: "TC001: Create a gadget"
description: "A case in a separate feature folder."
---
# TC001: Create a gadget
EOF
}

assert_rejected() {
  local bundle="$1"
  local expected="$2"
  local output="$bundle/check-output.txt"

  if python3 "$CHECKER" "$bundle" >"$output" 2>&1; then
    echo "expected checker to reject $bundle" >&2
    return 1
  fi
  grep -Fq "$expected" "$output"
}

python3 "$CHECKER" "$PROJECT_ROOT/knowledge"

valid_bundle="$TMP_DIR/valid"
write_bundle "$valid_bundle" "TC001: Create a widget" "TC002" "TC002: Delete a widget"
python3 "$CHECKER" "$valid_bundle"

duplicate_bundle="$TMP_DIR/duplicate"
write_bundle "$duplicate_bundle" "TC001: Create a widget" "TC001" "TC001: Delete a widget"
assert_rejected "$duplicate_bundle" "duplicate test case identifier TC001"

invalid_title_bundle="$TMP_DIR/invalid-title"
write_bundle "$invalid_title_bundle" "Create a widget" "TC002" "TC002: Delete a widget"
assert_rejected "$invalid_title_bundle" "test case title must start with 'TC###: '"
