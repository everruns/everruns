#!/usr/bin/env bash
# Real interpreter + Linux kernel isolation qualification. No model or network keys.
set -euo pipefail
cd "$(dirname "$0")/../.."
root="${EVERRUNS_MONTY_INSTALL_ROOT:-$PWD/.local/monty}"
mkdir -p "$root/bin"
if [[ ! -x "$root/bin/monty" ]] || [[ "$("$root/bin/monty" --version)" != "monty-runtime 1.5.0" ]]; then
  cargo install monty-runtime --version 1.5.0 --locked --root "$root"
fi
cargo build -p everruns-integrations --no-default-features --features monty --bin everruns-monty-worker
build_dir="$(cargo metadata --no-deps --format-version 1 | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
cp "$build_dir/debug/everruns-monty-worker" "$root/bin/everruns-monty-worker"
export EVERRUNS_MONTY_TEST_BIN="$root/bin/everruns-monty-worker"
cargo test -p everruns-integrations --no-default-features --features monty-isolation-tests,monty-runtime-tests --bin everruns-monty-worker --lib monty
