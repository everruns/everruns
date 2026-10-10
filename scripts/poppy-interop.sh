#!/usr/bin/env bash
# Run pi-poppy (github.com/richardanaya/pi-poppy), an independent Personal
# Agent Protocol ("Poppy") personal agent, against a Poppy channel.
#
# Poppy draft 0.1 has no official conformance suite. pi-poppy maps every
# MUST/SHOULD/MAY of the draft to its own tests, so its client code is the
# closest outside check of our wire format. It is checked out at a pinned
# commit under `.local/pi-poppy`, and the domain test `pi_poppy_interop_passes`
# serves a channel on a real socket and runs `scripts/poppy-interop/driver.ts`,
# which drives pi-poppy's `PersonalAgent` through discovery, a Session and a
# conversation. A stand-in runner answers every turn, so no model or worker is
# needed. The run ends with Datalayer's `personal-agent-protocol` discovery
# check (`pap company verify`). Needs git, Node 22+, npm, Python 3 and openssl.
set -euo pipefail

PI_POPPY_REPO="https://github.com/richardanaya/pi-poppy.git"
# Bump deliberately: a new commit can tighten checks.
PI_POPPY_COMMIT="11ea10630180740f16ccd507b8f90f69870c7e32"

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checkout="$root/.local/pi-poppy"

if [ ! -d "$checkout/.git" ]; then
  git clone --quiet "$PI_POPPY_REPO" "$checkout"
fi
git -C "$checkout" fetch --quiet origin "$PI_POPPY_COMMIT"
git -C "$checkout" checkout --quiet --detach "$PI_POPPY_COMMIT"
(cd "$checkout" && npm ci --silent --ignore-scripts)

# Datalayer's Python library (`pap company verify`) checks discovery too.
PAP_VERSION="0.3.2"
venv="$root/.local/pap-venv"
if [ ! -x "$venv/bin/python" ]; then
  python3 -m venv "$venv"
fi
"$venv/bin/pip" install --quiet "personal-agent-protocol==$PAP_VERSION"

cd "$root"
PAP_PYTHON="$venv/bin/python" PI_POPPY_DIR="$checkout" cargo test -p everruns-server --test domain \
  pi_poppy_interop_passes -- --nocapture --test-threads=1
