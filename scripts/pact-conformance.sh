#!/usr/bin/env bash
# Run PACT's own conformance suite (Identity profile, `e2e/pact.test.ts` in
# github.com/openpactprotocol/openpactprotocol) against an in-process server.
#
# The suite is checked out at a pinned commit under `.local/pact`, its
# dependencies installed with pnpm, and then the domain test
# `pact_conformance_suite_passes` serves two PACT channels on a real socket and
# runs the suite against them. A stand-in runner answers every turn, so no
# model or worker is needed. Needs git, Node 20+ and pnpm.
set -euo pipefail

PACT_REPO="https://github.com/openpactprotocol/openpactprotocol.git"
# Bump deliberately: the suite is the contract, so a new commit can add checks.
PACT_COMMIT="838c6bd1da9be39da04264e8b156dbb4e848a208"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checkout="$root/.local/pact"

if [ ! -d "$checkout/.git" ]; then
  git clone --quiet "$PACT_REPO" "$checkout"
fi
git -C "$checkout" fetch --quiet origin "$PACT_COMMIT"
git -C "$checkout" checkout --quiet --detach "$PACT_COMMIT"
(cd "$checkout" && pnpm install --frozen-lockfile --silent)

cd "$root"
PACT_E2E_DIR="$checkout/e2e" cargo test -p everruns-server --test domain \
  pact_conformance_suite_passes -- --nocapture
