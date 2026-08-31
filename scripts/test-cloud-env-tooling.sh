#!/usr/bin/env bash
# Guard scripts/init-cloud-env.sh against resolving a tool version through the
# GitHub API, or through a vendor installer that does it on the script's behalf.
#
# init-cloud-env.sh is the first thing a cloud-agent session runs, and `just` is
# what AGENTS.md requires before every push (`just pre-push`). So a tool that
# fails to install here does not merely degrade — it removes the pre-push gate
# from every cloud session that follows.
#
# `install_just` used to pipe https://just.systems/install.sh into bash. That
# installer resolves its version through
# `api.github.com/repos/casey/just/releases/latest`, and a cloud-agent session
# scopes the GitHub API to the repositories it was granted, so the call returns
# 403 for any other repo:
#
#   {"message": "GitHub access to this repository is not enabled for this
#    session. Use add_repo to request access."}
#
# The script then reported `One or more tool installs failed` and exited 1 with
# `just` absent, while gh, caddy and doppler installed fine — those three pin a
# version and fetch the release asset from github.com, which is not API-scoped.
# The comment on `install_doppler` already stated the rule ("Pinned version —
# skip GitHub API call"); `install_just` was the one installer that never got it.
#
# Checking only that `just` is pinned would not stop this recurring for the next
# tool, so this pins the property rather than the instance: no installer may
# reach api.github.com, and none may pipe a remote script into a shell — the two
# shapes that put version resolution back on a network path a cloud session
# cannot use. Both fail closed here instead of at 3am in someone's session.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
cd "$(dirname "$SCRIPT_DIR")"

TARGET="scripts/init-cloud-env.sh"

if [ ! -f "$TARGET" ]; then
  echo "FAIL: $TARGET is missing" >&2
  exit 1
fi

status=0

# Comments are allowed to name api.github.com — the explanation of why it is
# avoided lives in this file and in the script itself. Only code is checked.
CODE="$(sed 's/[[:space:]]*#.*$//' "$TARGET")"

# 1. No GitHub API dependency. Release assets on github.com are fine; the
#    api.github.com host is what a session's repo scoping refuses.
if grep -n 'api\.github\.com' <<<"$CODE"; then
  echo "FAIL: $TARGET reaches api.github.com — a cloud-agent session scopes the" >&2
  echo "      GitHub API to its granted repos and returns 403 for anything else." >&2
  echo "      Pin the version and fetch the release asset from github.com instead." >&2
  status=1
fi

# 2. No remote script piped into a shell. That hands version resolution to a
#    vendor installer, which is how the api.github.com call got in unnoticed.
if grep -nE 'curl[^|]*\|[[:space:]]*(ba)?sh' "$TARGET"; then
  echo "FAIL: $TARGET pipes a remote installer into a shell." >&2
  echo "      Vendor installers resolve versions over the GitHub API; pin the" >&2
  echo "      version and download the release asset directly." >&2
  status=1
fi

# 3. Every installer pins an explicit version. An unpinned tool has to ask
#    something what the latest release is, and the only thing to ask is the API
#    checked above.
for tool in just gh doppler caddy; do
  fn="install_${tool}"
  if ! grep -q "^${fn}()" "$TARGET"; then
    echo "FAIL: $TARGET has no ${fn} function" >&2
    status=1
    continue
  fi

  if ! awk "/^${fn}\\(\\)/,/^}/" "$TARGET" | grep -qE '_VERSION="[0-9]+\.[0-9]+'; then
    echo "FAIL: ${fn} does not pin an explicit version" >&2
    status=1
  fi
done

# 4. Checksum ratchet. `just` and `doppler` verify what they downloaded; `gh`
#    and `caddy` do not yet (EVE-945). This list may only grow — dropping a
#    verified tool back to unverified fails here.
for tool in just doppler; do
  if ! awk "/^install_${tool}\\(\\)/,/^}/" "$TARGET" | grep -q 'sha256sum -c'; then
    echo "FAIL: install_${tool} no longer verifies a sha256 checksum" >&2
    status=1
  fi
done

if [ "$status" -ne 0 ]; then
  exit 1
fi

echo "PASS: init-cloud-env.sh pins every tool and needs no GitHub API access"
