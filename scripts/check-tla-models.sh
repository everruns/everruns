#!/usr/bin/env bash
# Model-check the data plane v2 protocol models with TLC.
#
# Every model has a "safe" configuration that must pass and a "bug"
# configuration that reproduces the design flaw the model exists for (the
# funout move race, feed records keyed by Raft index alone, grants released
# on expiry, epochs fenced by holder name, the poison guard's progress
# rules). A bug configuration that stops failing means the model no
# longer checks what it claims to, so it fails this script too.
#
# Usage: scripts/check-tla-models.sh            (downloads TLC once)
#        TLA2TOOLS_JAR=/path/tla2tools.jar scripts/check-tla-models.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
MODELS="$ROOT/tests/models/data-plane-v2"
TLA_VERSION="1.7.4"
TLA_SHA256="936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88"
JAR="${TLA2TOOLS_JAR:-$ROOT/target/tla/tla2tools-$TLA_VERSION.jar}"

if [[ ! -f "$JAR" ]]; then
  mkdir -p "$(dirname "$JAR")"
  curl -sSfL -o "$JAR.tmp" \
    "https://github.com/tlaplus/tlaplus/releases/download/v$TLA_VERSION/tla2tools.jar"
  echo "$TLA_SHA256  $JAR.tmp" | sha256sum -c - >/dev/null
  mv "$JAR.tmp" "$JAR"
fi

# model  config  expected (pass | fail)
CHECKS=(
  "Moves Moves pass"
  "Moves MovesUnfenced fail"
  "Grants Grants pass"
  "Grants GrantsExpiryReleases fail"
  "Feed Feed pass"
  "Feed FeedByIndexOnly fail"
  "Epochs Epochs pass"
  "Epochs EpochsFencedByName fail"
  "Epochs EpochsCountEveryLease fail"
  "Epochs EpochsUnfencedProgress fail"
)

status=0
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
for check in "${CHECKS[@]}"; do
  read -r model config expected <<<"$check"
  out="$work/$config.out"
  if java -XX:+UseParallelGC -cp "$JAR" tlc2.TLC -deadlock -workers auto \
      -metadir "$work/$config.states" -config "$MODELS/$config.cfg" \
      "$MODELS/$model.tla" >"$out" 2>&1; then
    got=pass
  else
    got=fail
  fi
  if [[ "$got" == "$expected" ]]; then
    echo "ok   $config ($expected)"
  else
    echo "FAIL $config: expected $expected, got $got"
    grep -E 'Error|violated|line [0-9]+, col' "$out" | head -20 || true
    status=1
  fi
done
exit $status
