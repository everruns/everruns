set -e
source lib/rates.sh
for pair in '1 700' '1000 700' '1001 1200' '5000 1200' '5001 2400' '20000 2400' '20001 4800'; do
  read -r weight expected <<< "$pair"
  actual=$(quote "$weight" domestic)
  test "$actual" = "$expected" || { echo "FAIL: ${weight}g expected $expected, got $actual"; exit 1; }
  echo "PASS: ${weight}g -> ${actual} cents"
done
test "$(quote 1001 eu)" = 1650
test "$(quote 20001 international)" = 6600
if quote 100 unknown >/dev/null 2>&1; then echo 'FAIL: unknown zone accepted'; exit 1; fi
echo 'PASS: zone surcharges and unknown-zone rejection'
