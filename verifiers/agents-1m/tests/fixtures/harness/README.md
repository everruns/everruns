# Harness output fixtures

The files are constructed: they follow the exact shape each harness prints (the
`summary`/`result`/`res` dicts at the end of each script) with plausible numbers. Replace a file with a recorded output the
first time `verify --level local` runs that harness; `test_adapters.py`
also checks that every key an adapter reads still appears in its harness.
