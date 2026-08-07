# everruns-meta

Meta Model API provider for Everruns agents. It uses Meta's OpenAI-compatible
Responses API at `https://api.meta.ai/v1/responses`, supports server-managed
response history, and discovers Muse models from `/v1/models`.

```rust
use everruns_meta::MetaChatDriver;

let driver = MetaChatDriver::new("your-model-api-key");
```

See the [Meta provider guide](https://docs.everruns.com/providers/meta/).

## Live tests

The ignored live tests use the lower-cost Contributor model and require Meta's
standard `MODEL_API_KEY` environment variable:

```bash
doppler run -- cargo test -p everruns-meta --test model_api_live -- --ignored --nocapture
```
