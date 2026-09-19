cat > tests/run.sh <<'TESTS'
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

# Each tier edge, and the gram either side of it: this is where tiered pricing
# goes wrong.
check "at the 1000 g tier boundary" 700 "$(tier_rate_cents 1000)"
check "one gram over the 1000 g boundary" 1200 "$(tier_rate_cents 1001)"
check "at the 5000 g tier boundary" 1200 "$(tier_rate_cents 5000)"
check "one gram over the 5000 g boundary" 2400 "$(tier_rate_cents 5001)"
check "at the 20000 g tier boundary" 2400 "$(tier_rate_cents 20000)"
check "over the top tier pays the overweight rate" 4800 "$(tier_rate_cents 20001)"

check "zone surcharge is added on top of the tier" 1150 "$(quote 500 eu)"
check "domestic parcel pays its tier rate" 700 "$(quote 500 domestic)"

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
TESTS
echo "wrote tests/run.sh"
