---
title: OpenAI Image Generation
description: Generate and edit raster images with OpenAI's GPT Image API, persist artifacts, and save outputs into the session workspace.
---

| | |
|---|---|
| **ID** | `gpt_image_gen` |
| **Category** | Media |
| **Features** | None |
| **Dependencies** | [`session_file_system`](/capabilities/file-system/), [`session_storage`](/capabilities/session-storage/) |

Generate new raster images and edit existing ones with OpenAI's ChatGPT Images 2.0 API model, `gpt-image-2`, by default.

Capability config supports both model selection and a default quality used when the tool call does not specify one:

```json
{
  "model": "gpt-image-2",
  "default_quality": "medium",
  "partial_images": 1
}
```

If you need the previous generation model for compatibility, set `"model": "gpt-image-1"`.

The default quality is `medium`. That keeps latency and reliability reasonable for `gpt-image-2` while still producing polished outputs.

The default `partial_images` value is `1`. For single-image requests, the capability emits `tool.progress` status updates while waiting for the final image. Set it to `0` to disable progress updates, or up to `3` for more feedback at higher token cost.

This capability resolves credentials server-side, persists durable image artifacts, and can also write generated outputs into the session filesystem under `/workspace/.outputs/images/`.

## Credential Resolution

The tool layer never reads provider environment variables directly. Resolution order:

1. Session secret `OPENAI_API_KEY` (or `openai_api_key`)
2. Session secret `OPENAI_BASE_URL` (or `openai_base_url`) for endpoint override
3. Default OpenAI provider credentials from the control plane

Use [`secret_store`](/capabilities/session-storage/) for per-session overrides.

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
