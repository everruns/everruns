#!/usr/bin/env bash
# The whole suite. No framework, no interpreter, no network: `bash tests/run.sh`
# runs anywhere the repository does.
. lib/rates.sh

passed=0
failed=0

# check <name> <expected> <actual>
check() {
  if [ "$2" = "$3" ]; then
    echo "ok   $1"
    passed=$((passed + 1))
  else
    echo "FAIL $1: expected $2, got $3"
    failed=$((failed + 1))
  fi
}

check "domestic parcel costs the flat rate" 1200 "$(quote 500 domestic)"
check "zone surcharge is added" 1650 "$(quote 500 eu)"

if quote 500 moon >/dev/null 2>&1; then
  echo "FAIL unknown destination is rejected"
  failed=$((failed + 1))
else
  echo "ok   unknown destination is rejected"
  passed=$((passed + 1))
fi

echo
echo "$passed passed, $failed failed"
[ "$failed" -eq 0 ]
