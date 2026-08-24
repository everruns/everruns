# TC003: Answer Platform Questions from Embedded Docs

## Description

Verify that Platform Chat answers a repo-specific product question by consulting the embedded platform docs mounted at `/workspace/docs`, not by relying only on generic model knowledge.

This test exercises the `platform` capability's embedded-docs mount together
with the inherited file-system and bash tools from the `generic` harness. The
question is chosen so the correct answer depends on implementation details
documented in Everruns' own docs.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- A real LLM provider/model configured for the org (tool-calling capable). `llmsim-default` is not sufficient for validating docs lookup behavior.
- The signed-in user can start a chat thread on the built-in `platform-chat` harness from `/chats`
- Default org has the built-in `platform-chat` harness provisioned

## Test Data

| Field | Value |
|-------|-------|
| User prompt | `Using the embedded platform docs, tell me where Platform Chat can find Everruns documentation in the session filesystem, which file types are included in that mount, and whether those docs come from the repo at compile time or from database writes. Quote the docs path exactly.` |

## Steps

### Happy path — docs-backed answer

1. **Open Platform Chat** in the web UI: go to `/chats`, start a **New chat**, and pick the built-in **Platform Chat** harness.

2. **Send the prompt** from the test data table.

3. **Wait for Platform Chat to finish.**

4. **Inspect the response and tool trace.** The run should show evidence that Platform Chat consulted the mounted docs via file tools or the bashkit shell (for example `read_file`, `list_directory`, `grep`, or a `bash` call like `grep -r` / `cat /workspace/docs/...`).

5. **Verify the answer includes all three facts:**
   - The docs are available at `/workspace/docs`
   - Only markdown content is included (`.md` and `.mdx`)
   - The docs are embedded from the repo `docs/` directory at compile time rather than being written into the database per session

6. **Refresh the page** and confirm the answer remains in the Platform Chat thread transcript.

### Negative path A — wrong path

7. **Send:**

   ```text
   Are the embedded platform docs mounted at /workspace/specs?
   ```

8. **Expected:** Platform Chat corrects the path to `/workspace/docs` and does not confidently confirm `/workspace/specs`.

### Negative path B — unsupported file types

9. **Send:**

   ```text
   Do the embedded platform docs mount include JSON and PNG files from the repo docs directory?
   ```

10. **Expected:** Platform Chat answers no and explains that only markdown files are included in the virtual tree.

## Expected Result

### Docs Access

- Platform Chat consults the embedded docs mount when answering Everruns feature/configuration questions.
- Tool activity shows a docs lookup rather than a free-form answer with no retrieval evidence.

### Answer Correctness

- The answer explicitly names `/workspace/docs`.
- The answer states that included files are markdown only (`.md`, `.mdx`).
- The answer states that the source is the repo `docs/` directory embedded at compile time.
- The answer does not claim the docs are stored in session files or database rows.

### UX

- The answer is presented as a normal assistant message.
- Tool calls render as structured tool blocks, not raw JSON or `to=...` text.
- Refresh preserves the conversation in the Platform Chat thread.

## Failure Modes

| Failure | What to look for |
|---------|-----------------|
| No retrieval evidence | Platform Chat answers without consulting docs tools |
| Wrong mount path | Reply says `/docs`, `/workspace/specs`, or another incorrect path |
| Wrong file types | Reply claims JSON, images, or all repo files are embedded |
| Wrong storage model | Reply says docs are fetched from DB/session writes instead of compile-time embedding |
| Lost transcript on refresh | Global chat singleton or transcript persistence broken |

## Notes

- The exact wording may vary by model. Judge against the factual content, the docs lookup behavior, and the persisted transcript.
- This case is intentionally repo-specific. A correct answer should line up with `docs/capabilities/platform.md` and the `platform` capability implementation.
- If the environment falls back to `llmsim-default`, treat the run as invalid for this case: the simulated model returns canned text and does not prove embedded-docs retrieval.
