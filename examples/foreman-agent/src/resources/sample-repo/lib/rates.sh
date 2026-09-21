# Shipping rates.
#
# One flat rate per destination zone, applied to every parcel regardless of
# weight, which the pricing team has asked us to replace.
#
# Weights are whole grams, so every calculation here is integer arithmetic.

FLAT_RATE_CENTS=1200

# Surcharge in cents for a destination zone; fails for an unknown one.
zone_surcharge_cents() {
  case "$1" in
    domestic) echo 0 ;;
    eu) echo 450 ;;
    international) echo 1800 ;;
    *) return 1 ;;
  esac
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
  echo $((FLAT_RATE_CENTS + surcharge))
}
