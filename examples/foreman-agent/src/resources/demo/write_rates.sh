cat > src/rates.py <<'PY'
"""Shipping rates.

Weight tiers, applied per destination zone. A parcel is priced by the first
tier whose limit it does not exceed; anything heavier pays the top rate.
"""

WEIGHT_TIERS_CENTS = [
    (1.0, 700),
    (5.0, 1200),
    (20.0, 2400),
]
OVERWEIGHT_RATE_CENTS = 4800

ZONE_SURCHARGE_CENTS = {
    "domestic": 0,
    "eu": 450,
    "international": 1800,
}


def tier_rate_cents(weight_kg: float) -> int:
    """Return the tier rate in cents for a parcel weight."""
    for limit, cents in WEIGHT_TIERS_CENTS:
        if weight_kg <= limit:
            return cents
    return OVERWEIGHT_RATE_CENTS


def quote(weight_kg: float, destination: str) -> int:
    """Return the shipping cost in cents for a parcel."""
    if weight_kg <= 0:
        raise ValueError("weight_kg must be positive")
    if destination not in ZONE_SURCHARGE_CENTS:
        raise ValueError(f"unknown destination: {destination}")
    return tier_rate_cents(weight_kg) + ZONE_SURCHARGE_CENTS[destination]
PY
echo "wrote src/rates.py"
