---
title: OpenAI Image Generation
description: Generate and edit raster images with OpenAI's GPT Image API, persist artifacts, and save outputs into the session workspace.
---

| | |
|---|---|
| **ID** | `gpt_image_gen` |
| **Category** | Media |
| **Features** | None |
| **Dependencies** | [`session_file_system`](/capabilities/file-system/) |

Generate new raster images and edit existing ones with OpenAI's ChatGPT Images 2.0 API model, `gpt-image-2`, by default. The capability also supports Meta's Muse image model (`muse-image-1.0`) through Meta or OpenRouter providers.

Capability config supports both model selection and a default quality used when the tool call does not specify one:

```json
{
  "model": "gpt-image-2",
  "default_quality": "medium",
  "partial_images": 1,
  "fallback": "auto"
}
```

If you need the previous generation model for compatibility, set `"model": "gpt-image-1"`. To use the Muse image model instead, set `"model": "muse-image-1.0"` and configure a Meta provider (served as `muse-image-1.0`) or an OpenRouter provider (served as `meta/muse-image`).

When no OpenAI or Azure OpenAI credentials are configured but a Meta or OpenRouter provider is available, the capability falls back to the Muse image model if `fallback` is `"auto"` (the default). Set `fallback` to `"off"` to require OpenAI or Azure OpenAI credentials for GPT image models instead.

The default quality is `medium`. That keeps latency and reliability reasonable for `gpt-image-2` while still producing polished outputs.

The default `partial_images` value is `1`. For single-image requests, the capability emits `tool.progress` status updates while waiting for the final image. Set it to `0` to disable progress updates, or up to `3` for more feedback at higher token cost.

This capability resolves credentials server-side, persists durable image artifacts, and can also write generated outputs into the session filesystem under `/workspace/.outputs/images/`.

## Credential Resolution

The capability never reads provider credentials from session secrets or environment variables. Resolution order:

1. Default OpenAI provider credentials from the control plane
2. Default Azure OpenAI provider credentials from the control plane
3. Default Meta or OpenRouter provider credentials from the control plane (Muse image model)

## Tools

### `generate_image`

Generate one or more images from a prompt.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `prompt` | string | yes | Image generation prompt |
| `size` | enum | no | `1024x1024`, `1536x1024`, `1024x1536`, `auto` |
| `quality` | enum | no | `low`, `medium`, `high`, `auto`. Defaults to capability `default_quality`, which defaults to `medium` |
| `background` | enum | no | `transparent`, `opaque`, `auto` |
| `format` | enum | no | `png`, `jpeg`, `webp` |
| `count` | integer | no | Number of images to generate (1-10) |
| `save_to_session_fs` | boolean | no | Save images into the session filesystem |
| `output_dir` | string | no | Filesystem output directory (default `/workspace/.outputs/images`) |
| `filename_prefix` | string | no | Prefix for artifact and file names |
| `persist_artifact` | boolean | no | Persist into durable image storage (default `true`) |

### `edit_image`

Edit one or more existing images using a prompt.

| Parameter | Type | Required | Description |
|---|---|---|---|
| `prompt` | string | yes | Editing prompt |
| `image_id` | string | conditional | Durable image artifact ID to use as an edit source |
| `path` | string | conditional | Session filesystem path to use as an edit source |
| `size` | enum | no | `1024x1024`, `1536x1024`, `1024x1536`, `auto` |
| `quality` | enum | no | `low`, `medium`, `high`, `auto`. Defaults to capability `default_quality`, which defaults to `medium` |
| `background` | enum | no | `transparent`, `opaque`, `auto` |
| `format` | enum | no | `png`, `jpeg`, `webp` |
| `count` | integer | no | Number of images to produce (1-10) |
| `save_to_session_fs` | boolean | no | Save outputs into the session filesystem |
| `output_dir` | string | no | Filesystem output directory (default `/workspace/.outputs/images`) |
| `filename_prefix` | string | no | Prefix for artifact and file names |
| `persist_artifact` | boolean | no | Persist into durable image storage (default `true`) |

At least one of `image_id` or `path` is required. When both are present, both source images are sent to the edit request.

## Result Shape

Both tools return:

- Native image blocks for direct model consumption
- Structured JSON with:
  - `artifact_id` when durable storage is enabled
  - `session_file` when workspace save is enabled
  - `media_type`, `filename`, `size_bytes`
  - `revised_prompt` when OpenAI returns one

## Notes

- Transparent background requires `png` or `webp` output
- High quality can take substantially longer than medium or low on `gpt-image-2`
- Single-image requests emit progress updates by default; multi-image batches still wait for the final response
- Each additional streamed update adds extra image output tokens on the OpenAI side, so higher `partial_images` values trade cost for better perceived latency
- `generate_image` and `edit_image` stay fully exposed even when OpenAI `tool_search` is enabled, so large tool lists do not defer their schemas
- Session file edits must be `png`, `jpg`, `jpeg`, or `webp`
- Edit sources larger than 50 MB are rejected before the API call
- Saved workspace files are written as base64-encoded binary files

## See Also

- [File System](/capabilities/file-system/), read and reuse workspace images
- [Storage](/capabilities/session-storage/), store per-session OpenAI overrides
- [Capabilities Overview](/capabilities/)
