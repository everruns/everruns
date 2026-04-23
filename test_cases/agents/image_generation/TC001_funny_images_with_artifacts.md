# TC001: GPT Image Generation with Session Files and Persisted Artifacts

## Description

Verify that an agent using `gpt_image_gen` can complete a three-image generation request end to end: plan three `generate_image` calls, persist durable `img_*` artifacts, mirror each image into the session filesystem, and finish with a coherent final reply instead of an internal-error message.

This case is based on the real 2026-04-22 live run captured in `EVE-385`. It exists to preserve the intended contract independently of the regressions tracked in `EVE-383` and `EVE-384`.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- OpenAI credentials configured so `gpt_image_gen` can call `gpt-image-2` (default)
- OpenAI `gpt-5.4` model available in the org
- Agent can enable these capabilities:
  - `gpt_image_gen`
  - `session_file_system`
  - `session_storage`

## Test Data

| Field | Value |
|-------|-------|
| Agent name | `funny-images-20260421-210233` |
| Agent display name | `Funny Images Agent` |
| Agent description | `Generates funny images and saves them as artifacts.` |
| Model | `gpt-5.4` |
| Agent capabilities | `gpt_image_gen`, `session_file_system`, `session_storage` |
| Agent tags | `funny`, `images`, `dev-test` |
| Agent system prompt | `You create funny but safe images. When the user asks for images, use generate_image. Save outputs to /workspace/.outputs/images, persist image artifacts, and then reply with a short caption for each image plus any saved paths or image ids.` |
| Session title | `Funny image generation run` |
| Session tags | `funny`, `images`, `dev-test` |
| Session hints | `{"rich_media": true}` |
| Session max iterations | `10` |
| User prompt | `Generate three funny polished images with distinct jokes: 1) a raccoon presenting a quarterly earnings deck to pigeons in a boardroom, 2) a medieval knight trapped in a beige modern office cubicle during a boring standup, 3) a fluffy cat running a tiny neon-lit late-night diner for mice. Make them colorful and comedic. Save them to the session filesystem and persist artifacts. Then reply with a short caption for each image and mention the saved file paths or image ids.` |

## Steps

1. Create the agent with the exact fixture values above, including `gpt-5.4`, the three listed capabilities, and `max_iterations: 10`.

2. Create a new session on that agent with:
   - title `Funny image generation run`
   - tags `funny`, `images`, `dev-test`
   - hints `{"rich_media": true}`
   - `max_iterations: 10`

3. Send the user prompt exactly as listed in **Test Data**.

4. Wait for the session to return to `idle`.

5. Inspect the session messages and event stream:
   - confirm the agent emitted three `generate_image` tool calls
   - confirm each tool call completed successfully
   - confirm the final assistant message is present

6. Inspect the session filesystem under `/workspace/.outputs/images` and verify that three PNG files were written there.

7. Inspect the Images API and verify that three retrievable `img_*` artifacts exist for this run.

8. Perform a manual visual check of the rendered outputs and confirm the three concepts are distinct:
   - raccoon presenting earnings to pigeons in a boardroom
   - knight trapped in a beige office cubicle during standup
   - fluffy cat running a neon-lit diner for mice

## Expected Result

### Session Lifecycle

- Session reaches `idle`
- Event stream includes `turn.started`, `reason.completed`, `turn.completed`, and `session.idled`
- No `tool.error`, `internal_error`, or equivalent failure event appears

### Tool Planning

- Exactly 3 `generate_image` tool calls are emitted
- Every call includes:
  - `save_to_session_fs: true`
  - `persist_artifact: true`
  - `format: png`
  - `size: 1536x1024`
  - `quality: high`
  - `background: opaque`
- The three calls target the three requested joke concepts and use distinct filename prefixes

### Final Reply

- Final assistant reply contains a short caption for each of the three images
- Final assistant reply mentions the saved file paths and/or the persisted `img_*` artifact IDs
- Final assistant reply does not claim the run failed or that nothing was saved

### Session Filesystem

- Three PNG files exist under `/workspace/.outputs/images/`
- Each saved file has a one-to-one match with a returned image artifact
- For the recorded 2026-04-22 fixture, the expected file paths were:
  - `/workspace/.outputs/images/funny_raccoon_boardroom.png`
  - `/workspace/.outputs/images/funny_knight_cubicle.png`
  - `/workspace/.outputs/images/funny_cat_diner.png`

### Image Artifacts

- Three image artifacts exist and are retrievable through `/v1/images/{id}`
- Artifact IDs use the round-trippable `img_*` format
- Each artifact is a PNG and matches the corresponding session file

### Visual Distinctness

- The three generated images are visibly different from one another
- Each image matches its requested joke concept closely enough that a human reviewer can identify it without relying on the filename

## Failure Modes

| Failure | What to look for |
|---------|-----------------|
| False failure after successful generation | Final assistant text says the run hit an internal error or that nothing was saved even though files or artifacts exist |
| Artifact IDs not reusable | Returned IDs are not `img_*` values or `/v1/images/{id}` cannot fetch them back |
| Filesystem mirror missing | Artifacts exist but `/workspace/.outputs/images/` is missing one or more PNG files |
| Tool-call mismatch | Fewer than 3 `generate_image` calls, wrong output format/size/quality, or `persist_artifact` / `save_to_session_fs` omitted |
| Collapsed concepts | Images are duplicates or do not clearly map to raccoon-boardroom, knight-cubicle, and cat-diner concepts |

## Recorded Fixture Evidence (2026-04-22)

- Agent id: `agent_019db2ecc0017331a0ec8fe20dff9ad6`
- Session id: `session_019db2ed04fb71eda72ce6eb262b41b1`
- Recorded image artifacts:
  - `funny_raccoon_boardroom.png` -> `img_019db2ee211d7398ad2ba9871e3f466d`
  - `funny_knight_cubicle.png` -> `img_019db2ee1c8d7adaaad316c1ee169016`
  - `funny_cat_diner.png` -> `img_019db2ee35e974e78c0151b472e2ab49`
- Recorded prompt themes:
  - raccoon boardroom: polished comedic corporate satire with a raccoon presenting quarterly earnings to pigeons
  - knight cubicle: polished comedic fantasy-vs-office contrast with a knight trapped in a beige cubicle during standup
  - cat diner: polished comedic neon late-night diner run by a fluffy cat for mice

## Regression Note

The 2026-04-22 live run incorrectly ended with:

`I hit an internal error while trying to generate all three images, so nothing was saved yet.`

That message was wrong because the image artifacts did exist. Treat that text as regression evidence only. The correct contract for this test case is that the run completes successfully, returns usable file paths and/or image IDs, and leaves three retrievable images behind.
