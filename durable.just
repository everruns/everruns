# Durable execution benchmarks

# Run benchmarks (in-memory)
bench *args:
    ./scripts/lib/bench.sh memory {{ args }}

# Run benchmarks (PostgreSQL)
bench-db *args:
    ./scripts/lib/bench.sh db {{ args }}
