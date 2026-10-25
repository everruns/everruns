#!/usr/bin/env bash
# Run `everruns docker up` end to end with a CLI built from this checkout:
# first run (no prompts), CLI signed in, a second run that changes nothing,
# a sign-in mode switch, stop, an upgrade with its backup, and reset.
#
# Uses published images like the Compose smoke (EVERRUNS_TAG, default
# `latest`); the CLI under test is the local build. Settings and credentials go
# to a temporary XDG home so the runner's own profiles are never touched.
#
# Usage: scripts/smoke-everruns-docker.sh [path-to-everruns-binary]
#   KEEP_STACK=1 leaves the stack running for inspection.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BIN="${1:-$ROOT/target/debug/everruns}"
TAG="${EVERRUNS_TAG:-latest}"
NAME="smoke"

TMP="$(mktemp -d)"
export XDG_CONFIG_HOME="$TMP/config" XDG_DATA_HOME="$TMP/data"
export RUST_BACKTRACE=0
unset EVERRUNS_API_KEY EVERRUNS_API_URL EVERRUNS_PROFILE

step() { printf '\n==> %s\n' "$*"; }
fail() {
  echo "::error title=everruns docker smoke::$*" >&2
  "$BIN" docker logs --name "$NAME" --no-follow server >&2 || true
  exit 1
}
cleanup() {
  if [ "${KEEP_STACK:-}" != "1" ]; then
    docker compose --project-name "everruns-$NAME" down -v --remove-orphans >/dev/null 2>&1 || true
    rm -rf "$TMP"
  fi
}
trap cleanup EXIT

step "Docker checks"
"$BIN" docker doctor --name "$NAME" || fail "doctor failed"

step "First run, no prompts"
"$BIN" docker up --name "$NAME" --yes --image-tag "$TAG" --provider skip --admin-email smoke@example.com \
  || fail "first up failed"

step "CLI is signed in to the local instance"
"$BIN" whoami | grep -q "smoke@example.com" || fail "whoami is not the local admin"
"$BIN" agents list -o json >/dev/null || fail "agents list failed"

step "Second run changes nothing"
env_before="$(sha256sum "$XDG_DATA_HOME/everruns/docker/$NAME/.env")"
started="$(date +%s)"
"$BIN" docker up --name "$NAME" --yes 2>&1 | tee "$TMP/second.log"
elapsed=$(( $(date +%s) - started ))
[ "$env_before" = "$(sha256sum "$XDG_DATA_HOME/everruns/docker/$NAME/.env")" ] || fail ".env changed on a second run"
grep -q "Recreate" "$TMP/second.log" && fail "a second run recreated containers"
echo "second run took ${elapsed}s"

step "Switch to no sign-in and back"
"$BIN" docker up --name "$NAME" --yes --auth none || fail "switch to none failed"
docker port "everruns-$NAME-caddy-1" 9300 | grep -q '^127.0.0.1:' || fail "no sign-in is reachable beyond 127.0.0.1"
"$BIN" docker up --name "$NAME" --yes --auth single || fail "switch back failed"
"$BIN" whoami | grep -q "smoke@example.com" || fail "CLI not signed in again after leaving no sign-in"

step "Status and stop"
"$BIN" docker status --name "$NAME" | grep -q "Health:   healthy" || fail "status is not healthy"
"$BIN" docker stop --name "$NAME" || fail "stop failed"
"$BIN" docker stop --name "$NAME" || fail "a second stop failed"

step "Start again after stop"
"$BIN" docker up --name "$NAME" --yes || fail "up after stop failed"

step "Upgrade backs up the database and keeps the CLI signed in"
# `latest` moves under one name, so upgrading to it always pulls and backs up;
# a release tag the instance already runs is a no-op.
"$BIN" docker upgrade --name "$NAME" --image-tag "$TAG" 2>&1 | tee "$TMP/upgrade.log"
[ "${PIPESTATUS[0]}" = 0 ] || fail "upgrade failed"
if [ "$TAG" = "latest" ]; then
  ls "$XDG_DATA_HOME/everruns/docker/$NAME/backups/"*.dump >/dev/null 2>&1 || fail "upgrade took no backup"
else
  grep -q "Already on $TAG" "$TMP/upgrade.log" || fail "upgrade to the running tag was not a no-op"
fi
"$BIN" whoami | grep -q "smoke@example.com" || fail "CLI not signed in after the upgrade"

step "Reset deletes everything, and only after --yes"
"$BIN" docker reset --name "$NAME" </dev/null && fail "reset went ahead without --yes"
"$BIN" docker reset --name "$NAME" --yes || fail "reset failed"
[ -e "$XDG_DATA_HOME/everruns/docker/$NAME" ] && fail "reset left the settings folder"
docker volume ls -q --filter "label=com.docker.compose.project=everruns-$NAME" | grep -q . && fail "reset left volumes"
grep -q "local-$NAME" "$XDG_CONFIG_HOME/everruns/credentials.json" && fail "reset left the CLI profile"
"$BIN" docker reset --name "$NAME" --yes || fail "a second reset failed"

step "everruns docker smoke passed"
