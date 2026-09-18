"""Shipping rates.

One flat rate per destination zone, applied to every parcel regardless of
weight, which the pricing team has asked us to replace.
"""

FLAT_RATE_CENTS = 1200

ZONE_SURCHARGE_CENTS = {
    "domestic": 0,
    "eu": 450,
    "international": 1800,
}


def quote(weight_kg: float, destination: str) -> int:
    """Return the shipping cost in cents for a parcel."""
    if weight_kg <= 0:
        raise ValueError("weight_kg must be positive")
    if destination not in ZONE_SURCHARGE_CENTS:
        raise ValueError(f"unknown destination: {destination}")
    return FLAT_RATE_CENTS + ZONE_SURCHARGE_CENTS[destination]
