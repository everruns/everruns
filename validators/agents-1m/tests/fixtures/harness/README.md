# Harness output fixtures

`memory_bench.py` is read from its committed baseline
(`scripts/agentd/memory_bench_baseline.json`, a real recorded run). The other
files are constructed: they follow the exact shape each harness prints (the
`summary`/`result`/`res` dicts at the end of each script) with plausible numbers. Replace a file with a recorded output the
first time `validate --level local` runs that harness; `test_adapters.py`
also checks that every key an adapter reads still appears in its harness.
