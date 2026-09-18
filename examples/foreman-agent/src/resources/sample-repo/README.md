# shipkit

Shipping cost calculation for the storefront.

`quote(weight_kg, destination)` returns the cost in cents. Rates are flat today:
every parcel costs the same regardless of weight, which is why heavy parcels
lose money and light ones are overpriced.

Run the tests with `python -m pytest`.
