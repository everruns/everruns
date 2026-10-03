#!/usr/bin/env bash
# Effectful source owners are excluded only from kernel scans; the kernel guard
# separately verifies their feature gates in core/src/lib.rs.
core_kernel_source_files() {
  find crates/core/src     \( -path crates/core/src/host -o -path crates/core/src/mcp -o -path crates/core/src/ag_ui \) -prune -o     -name '*.rs' ! -path crates/core/src/a2a.rs -type f -print
}

assert_core_feature_module_gates() {
  python3 - "${1:-crates/core/src/lib.rs}" <<'PY_GATE'
import re
import sys
from pathlib import Path
source = Path(sys.argv[1]).read_text()
for module, feature in [("host", "host"), ("engine", "engine"), ("builtins", "builtins"), ("mcp", "mcp"), ("ag_ui", "ag-ui"), ("a2a", "a2a")]:
    pattern = r'#\[cfg\(feature = "' + re.escape(feature) + r'"\)\]\s*(?:(?:#\[[^\n]*\]|//[^\n]*)\s*)*pub mod ' + module + ';'
    if not re.search(pattern, source):
        raise SystemExit(f"core::{module} must stay behind feature {feature}")
PY_GATE
}
