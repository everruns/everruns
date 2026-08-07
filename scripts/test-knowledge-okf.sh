#!/usr/bin/env bash
# Verify the canonical knowledge bundle's OKF v0.2 structure and links.

set -euo pipefail

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
python3 "$PROJECT_ROOT/scripts/check_okf.py" "$PROJECT_ROOT/knowledge"
