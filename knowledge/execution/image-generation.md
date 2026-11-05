---
type: Specification
title: "Image Generation"
description: "The image_generation capability: model resolution (agent, org default, platform default), typed images driver over host egress, GIF and SVG outputs, and how Framework hosts differ."
tags:
  - everruns
  - execution
  - capabilities
  - images
---

# Image Generation

Status: implemented. Capability `image_generation`, source
`crates/integrations/src/image_generation/`; driver
`crates/drivers/drivers/src/openai/images.rs`; contract
`crates/contracts/src/images.rs`. Public page: `docs/capabilities/image-generation.md`.

## Why

Image work used to be one capability hard-wired to OpenAI (`gpt_image_gen`) with its own
HTTP client and a fixed model. GPT Image 2.6 added animated GIFs, SVG, `complexity` and
markup-guided edits, and organizations reach image models through several providers
(OpenAI, Azure OpenAI, OpenRouter, Meta). The capability became provider-neutral, the
model became a catalog choice like chat and decision models, and the wire code moved
into a typed driver so the Framework and server share it.

## Model resolution

First match wins:

1. The capability's `model` config. A catalog id (`model_…`) binds that exact enabled
   org model through `get_image_model(Some(id))`, with no fallback: a wrong choice must
   fail loudly, not draw with something else. A plain model name goes through the
   provider-type path below.
2. The organization default image model (`image_model_defaults`, `/v1/models/images-default`),
   via `get_image_model(None)`. Setting it requires an enabled model whose service is `images`.
3. `PLATFORM_DEFAULT_IMAGE_MODEL` (`gpt-image-2.6-flare`).
4. With `fallback: auto`, Muse Image on Meta or OpenRouter when no provider serves the
   GPT Image model.

The provider-type path tries provider types in order (GPT Image: OpenAI, Azure OpenAI,
OpenRouter; Muse: Meta, OpenRouter) and skips a provider whose masked profile lacks the
requested format. `mask_for_surface` strips GIF, SVG, markup and complexity from
non-OpenAI/Azure surfaces because OpenRouter serves GPT Image 2.6 as raster stills only.

## Request shaping

`build_request` validates every call against the resolved `ImageModelProfile`:

- GIF and SVG return one image. GIF `size` is dropped: the frame count picks the sheet
  size, and an explicit size is rejected by the API.
- `frames` and `frame_duration_ms` are GIF-only and checked against `GifLimits`.
- A configured quality or complexity the model lacks is dropped (the model default
  applies); one the call asked for explicitly is a tool error, so the model can retry.
- Partial images stream only for a single still.

## Outputs

- Stills and GIFs become image artifacts unless `persist_artifact` is false. The chat
  model sees a GIF as its first frame in PNG, since model APIs take stills; people see
  the animation (UI plays the artifact and pauses on that frame).
- SVGs never become image artifacts (an SVG served from the image endpoints could carry
  script). They are written to the session filesystem only.
- Without an image store (Framework hosts) images are written to session files.
- `output_dir` config sets the folder. Framework workspace policy keeps hidden paths
  read-only, so the default `/workspace/.outputs/images` cannot be written there; the
  Framework example and `serve` `[images]` use `/workspace/images`.
- Provider errors and file-save denials are tool errors; malformed provider output is an
  internal error.

## Egress

All traffic goes through `EgressImagesTransport` over the host `EgressService` with the
session ACL, DNS pinning and no redirects (TM-LLM-047, EVE-1174). No egress service means
the tools fail closed.

## Defaults

Worker kit agents and platform chat carry the capability. The deprecated Generic harness
keeps its legacy set. Framework apps opt in with the `images` feature, which registers the
capability with environment credentials (`EnvImageCredentials`).

## Tests

- `crates/integrations/src/image_generation/capability_tests.rs`, `egress_tests.rs`
- `crates/drivers/drivers/src/openai/images_tests.rs`
- Live: `crates/llm-tests/tests/provider_live/openai_images.rs`
- Example: `crates/everruns/examples/image_generation.rs`
