import pytest

from src.rates import quote


def test_domestic_parcel_costs_the_flat_rate():
    assert quote(1.0, "domestic") == 1200


def test_zone_surcharge_is_added():
    assert quote(1.0, "eu") == 1650


def test_unknown_destination_is_rejected():
    with pytest.raises(ValueError):
        quote(1.0, "moon")
