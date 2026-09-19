cat > lib/rates.sh <<'RATES'
# Shipping rates.
#
# Weight tiers, applied per destination zone. A parcel is priced by the first
# tier whose limit it does not exceed; anything heavier pays the top rate.
#
# Weights are whole grams, so every calculation here is integer arithmetic.

# Surcharge in cents for a destination zone; fails for an unknown one.
zone_surcharge_cents() {
  case "$1" in
    domestic) echo 0 ;;
    eu) echo 450 ;;
    international) echo 1800 ;;
    *) return 1 ;;
  esac
}

# Tier rate in cents for a parcel weight: tier_rate_cents <weight_g>
tier_rate_cents() {
  if [ "$1" -le 1000 ]; then
    echo 700
  elif [ "$1" -le 5000 ]; then
    echo 1200
  elif [ "$1" -le 20000 ]; then
    echo 2400
  else
    echo 4800
  fi
}

# Shipping cost in cents: quote <weight_g> <destination>
quote() {
  case "$1" in
    ''|*[!0-9]*) echo "weight_g must be a whole number of grams" >&2; return 1 ;;
  esac
  surcharge="$(zone_surcharge_cents "$2")" || {
    echo "unknown destination: $2" >&2
    return 1
  }
  echo $(( $(tier_rate_cents "$1") + surcharge ))
}
RATES
echo "wrote lib/rates.sh"
