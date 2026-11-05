# everruns-integrations

This integration is shipped as the `image_generation` module of `everruns-integrations`. Enable it
with `features = ["image-generation"]` in Cargo.

> Image generation and editing for Everruns agents: stills, animated GIFs and SVGs.

Part of the [Everruns](https://everruns.com) ecosystem.

## What It Provides

Exposes the `image_generation` capability with two tools:

- `generate_image`, draw stills, animated GIFs (`format: "gif"`) or SVGs from a prompt.
- `edit_image`, edit existing images, optionally guided by a markup layer.

The model is the agent's `model` setting, else the organization's default image model, else
GPT Image 2.6 Flare. Requests go through the host's egress service with the typed
`OpenAIImagesDriver` from `everruns-drivers`.

```rust
use everruns_integrations::image_generation::ImageGenerationCapability;

let _capability = ImageGenerationCapability::new();
```

The capability publishes an `IntegrationPlugin` const named by the integration catalog, so
linking this crate into a binary (e.g. `everruns-server`, `everruns-worker`) is
enough to make it available. An embedding application without a provider credential store
passes its own with `ImageGenerationCapability::with_credentials`.

## Documentation

- [Image generation](https://everruns.com/docs/capabilities/image-generation/)
- [API reference](https://docs.rs/everruns-integrations)

## License

Licensed under the [MIT License](https://github.com/everruns/everruns/blob/main/LICENSE).
