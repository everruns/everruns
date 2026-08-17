# everruns-integrations-openai-image

> OpenAI GPT Image generation and editing for Everruns agents.

Part of the [Everruns](https://everruns.com) ecosystem.

## What It Provides

Exposes the `gpt_image_gen` capability with two tools:

- `generate_image`, create raster images from a text prompt.
- `edit_image`, edit existing session images with OpenAI's image edit API.

```rust
use everruns_integrations_openai_image::GptImageGenCapability;

let _capability = GptImageGenCapability;
```

The capability auto-registers via the inventory `IntegrationPlugin` system, so
linking this crate into a binary (e.g. `everruns-server`, `everruns-worker`) is
enough to make it available.

Per-session overrides for the OpenAI API key and base URL are read from the
session secret store as `OPENAI_API_KEY` and optionally `OPENAI_BASE_URL`.

## Documentation

- [OpenAI image generation](https://docs.everruns.com/capabilities/openai-image-generation/)
- [API reference](https://docs.rs/everruns-integrations-openai-image)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
