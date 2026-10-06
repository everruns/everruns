# everruns-integrations

This integration is shipped as the `openai-decisions` module of `everruns-integrations` Enable it with `features = ["openai-decisions"]` in Cargo.

OpenAI's Decisions API as an Everruns decision driver (`openai`). Preview:
the API is in limited preview and its wire shape is not yet published, so the
request and response types in `src/wire.rs` are inferred and the crate is not
published. See `knowledge/operations/decisions-service.md`.

```bash
cargo test -p everruns-integrations --features openai-decisions
# Needs an account with Decisions API access:
doppler run -- cargo test -p everruns-integrations --features openai-decisions-live-tests
```
