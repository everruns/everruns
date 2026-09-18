cat > tests/test_tiers.py <<'PY'
import pytest

from src.rates import quote, tier_rate_cents


@pytest.mark.parametrize(
    "weight_kg, cents",
    [(0.5, 700), (1.0, 700), (1.01, 1200), (5.0, 1200), (5.01, 2400)],
)
def test_tier_boundaries_are_inclusive_at_the_limit(weight_kg, cents):
    assert tier_rate_cents(weight_kg) == cents


def test_parcels_above_the_top_tier_pay_the_overweight_rate():
    assert tier_rate_cents(20.01) == 4800


def test_the_zone_surcharge_is_still_added_on_top_of_the_tier():
    assert quote(1.0, "eu") == 700 + 450
PY
sed -i 's/^def test_domestic_parcel_costs_the_flat_rate():$/def test_domestic_parcel_costs_its_tier_rate():/' tests/test_rates.py
sed -i 's/    assert quote(1.0, "domestic") == 1200/    assert quote(1.0, "domestic") == 700/' tests/test_rates.py
sed -i 's/    assert quote(1.0, "eu") == 1650/    assert quote(1.0, "eu") == 1150/' tests/test_rates.py
echo "wrote tests/test_tiers.py and updated tests/test_rates.py"
