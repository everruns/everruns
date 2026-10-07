# everruns-integrations

This integration is shipped as the `openai-decisions` module of `everruns-integrations` Enable it with `features = ["openai-decisions"]` in Cargo.

OpenAI's [Decisions API](https://developers.openai.com/api/docs/guides/decisions)
as an Everruns decision driver (`openai`). The wire types follow the published
reference. See `knowledge/operations/decisions-service.md`.

```bash
cargo test -p everruns-integrations --features openai-decisions
# Live smoke against the real API:
doppler run -- cargo test -p everruns-integrations --features openai-decisions-live-tests
```
