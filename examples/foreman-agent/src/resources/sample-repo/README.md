# shipkit

Shipping cost calculation for the storefront, in portable shell.

`quote <weight_g> <destination>` prints the cost in cents. Weights are whole
grams. Rates are flat
today: every parcel costs the same regardless of weight, which the pricing team
has asked us to replace.

Run the tests with `bash tests/run.sh`. They need nothing installed.
