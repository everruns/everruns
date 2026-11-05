---
title: Image Generation
description: Generate and edit images, animated GIFs and SVGs with GPT Image 2.6 or any image model your organization has, and keep them as artifacts or workspace files.
appliesTo: [platform, cloud, framework]
---

| | |
|---|---|
| **ID** | `image_generation` |
| **Category** | Media |
| **Features** | None |
| **Dependencies** | None |

Image Generation gives an agent two tools, `generate_image` and `edit_image`. It draws stills (PNG, JPEG, WebP), animated GIFs and SVG vector images. New agents built from the Worker kit get it by default, and so does platform chat.

The default model is [GPT Image 2.6 Flare](https://openai.com/index/introducing-images-2-6/). GPT Image 2.6 Sunburst, earlier GPT Image models and Meta's Muse Image work too.

## Choosing the model

The model comes from the first of these that is set:

1. The capability's `model` setting on the agent. It accepts a model from the organization's catalog (pick one in the agent's capability settings) or a model name such as `gpt-image-2.6-sunburst`.
2. The organization's **Default image model**, set under **Models → Defaults → Images** or with `everruns models images-default set <model_id>`.
3. GPT Image 2.6 Flare.

Image models appear on the Models page under the **Images** filter, with GIF, SVG and Edit badges showing what each one supports.

A model name is served by the first provider that has it: OpenAI, then Azure OpenAI, then OpenRouter for GPT Image models, and Meta or OpenRouter for Muse Image. A provider that cannot produce the requested format is skipped: OpenRouter serves GPT Image 2.6 as PNG, JPEG and WebP only, so GIF and SVG need an OpenAI or Azure OpenAI provider. When no provider serves the chosen GPT Image model and `fallback` is `"auto"` (the default), the capability uses Muse Image on a Meta or OpenRouter provider instead.

## Settings

```json
{
  "model": "gpt-image-2.6-flare",
  "default_format": "png",
  "default_quality": "medium",
  "complexity": "low",
  "gif_frames": 9,
  "gif_frame_ms": 160,
  "partial_images": 1,
  "output_dir": "/workspace/images",
  "fallback": "auto"
}
```

| Setting | Meaning |
|---|---|
| `model` | Image model. Empty means the organization default. |
| `default_format` | `png` (default), `jpeg`, `webp`, `gif` or `svg`, when a call does not choose. |
| `default_quality` | `medium` (default), `low`, `high`, `xhigh`, `max` or `auto`. `xhigh` and `max` need GPT Image 2.5 or newer; a model without them uses its own default. |
| `complexity` | `low`, `medium` or `high`: how much detail GPT Image 2.5 and newer put into a scene. Lower is faster. |
| `gif_frames` | Frames in a GIF, 2 to 64. The model default is 9. |
| `gif_frame_ms` | How long each GIF frame shows, 20 to 2000 ms. The model default is 200 ms. |
| `partial_images` | 0 to 3 streamed progress updates for a single still image. |
| `output_dir` | Session folder for saved images. Defaults to `/workspace/.outputs/images`. |
| `fallback` | `auto` or `off`, see above. |

## Animated GIFs

Ask for `format: "gif"` and describe the motion in the prompt. The model draws every frame in one request, so a GIF costs one call. The frame count sets the frame size, so `size` is ignored for GIFs. GIFs are generated, not edited, and each call makes one.

The GIF is stored as an image artifact, and chat plays it with its frame count and loop length, a pause button and a download link. Chat models only accept still images, so the agent itself sees the first frame.

## SVG

`format: "svg"` returns one vector image. SVGs are never kept as image artifacts, since an SVG served from the image endpoints could carry script. They are always written to the session filesystem and the tool result gives the path.

## Tools

### `generate_image`

| Parameter | Type | Description |
|---|---|---|
| `prompt` | string | What to draw. For a GIF, describe the motion. Required. |
| `format` | enum | `png`, `jpeg`, `webp`, `gif`, `svg` |
| `frames` | integer | GIF frames, 2 to 64 |
| `frame_duration_ms` | integer | GIF frame duration, 20 to 2000 ms |
| `quality` | enum | `low`, `medium`, `high`, `xhigh`, `max`, `auto` |
| `complexity` | enum | `low`, `medium`, `high` |
| `size` | enum | `1024x1024`, `1536x1024`, `1024x1536`, `auto` |
| `background` | enum | `transparent`, `opaque`, `auto` |
| `count` | integer | 1 to 10 stills. GIF and SVG make one. |
| `save_to_session_fs` | boolean | Also save each image to the session filesystem |
| `output_dir` | string | Session folder for saved images |
| `filename_prefix` | string | Filename prefix |
| `persist_artifact` | boolean | Keep each image as an artifact (default `true`) |

### `edit_image`

Takes the same parameters except `frames` and `frame_duration_ms`, plus:

| Parameter | Type | Description |
|---|---|---|
| `prompt` | string | The edit to make. Required. |
| `image_id` | string | Image artifact to edit |
| `path` | string | Session file to edit |
| `markup_path` | string | A transparent PNG with drawn marks (circles, arrows, notes) showing where to change the image. GPT Image 2.6 only. |

At least one of `image_id` or `path` is required.

## Result

Both tools return the image for the model to see (the first frame for a GIF, nothing for an SVG) and JSON with the provider, model, format and token usage, and for each image its `artifact_id`, `session_file`, `filename`, `media_type`, `size_bytes`, `frames` and `revised_prompt` when present.

## Credentials and network

Hosted Everruns resolves provider accounts on the server; the capability never reads provider keys from session secrets. In the [Framework](/framework/), enable the `images` feature and the capability reads `OPENAI_API_KEY` (and `OPENAI_BASE_URL`), `AZURE_OPENAI_API_KEY` with `AZURE_OPENAI_BASE_URL`, `OPENROUTER_API_KEY`, or `META_API_KEY` from the environment.

Image requests leave through the platform's outbound network boundary, like web fetch and MCP. The provider base URL must be allowed by the session's network access policy and, when enabled, the system allowlist. Its hostname must resolve to public addresses only, and the connection is pinned to those addresses. Redirects from the image API are errors and never followed, so the provider key is only sent to the configured host.

## Notes

- Transparent backgrounds need PNG, WebP or GIF
- Higher quality and complexity take longer
- Edit sources must be PNG, JPEG or WebP and at most 50 MB
- `generate_image` and `edit_image` stay fully exposed when OpenAI `tool_search` is on

## See Also

- [File System](/capabilities/file-system/), read and reuse workspace images
- [Framework example](https://github.com/everruns/everruns/blob/main/crates/everruns/examples/image_generation.rs), an agent that saves a GIF next to you
- [Capabilities Overview](/capabilities/)
