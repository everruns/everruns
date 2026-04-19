# TC001: A2UI Form Rendering and Submission

## Description

Verify that an agent with the **A2UI** capability enabled can emit a ```a2ui fenced JSON block describing a `Form` with multiple fields, that the chat UI renders the form using native shadcn/ui primitives, and that submitting the form sends a new chat message back to the agent summarizing the collected field values.

Exercises the full A2UI round-trip for interactive UI: system-prompt catalog instruction, LLM JSON emission, fenced-block detection, streaming-tolerant parsing, native-primitive rendering, form-state collection via React context, and `message` action dispatch through the existing chat pipeline.

## Preconditions

- Control-plane running (`just start-dev` or `just start-all`)
- LLM API key configured (OpenAI, Anthropic, or Gemini — catalog lives in the system prompt, so any frontier chat model works)
- Branch with the `a2ui` capability registered in `CapabilityRegistry::with_builtins_for_grade`

## Test Data

| Field | Value |
|-------|-------|
| Agent name | A2UI Form Demo |
| Harness | `base` (or any harness — A2UI is harness-agnostic) |
| Capabilities | `a2ui` (enabled), no other UI capabilities |
| Model | Any frontier chat model that reliably emits structured JSON |

### Prompt

```
Render a feedback form with the following fields:
- "name" — single-line text, required, label "Your name"
- "email" — single-line text, label "Email"
- "rating" — dropdown with options 1..5
- "comments" — multi-line text, label "Comments", 4 rows
- a "subscribe" checkbox, default checked, label "Send me product updates"

Use the submit label "Send feedback". When the user submits, echo back a summary.
```

## Steps

1. **Create an agent** named "A2UI Form Demo". In the capability picker, enable **A2UI** and confirm no other generative-UI capability (e.g. OpenUI) is enabled.

2. **Start a session** on that agent.

3. **Send the prompt above.**

4. **Wait for the agent to finish.** The response should stream into the chat and contain exactly one ```a2ui fenced code block describing a `Form` with the five requested fields.

5. **Inspect the rendered form** in the chat message. Confirm each field appears with the requested label, the dropdown lists options `1` through `5`, and the checkbox is pre-checked.

6. **Fill in values:**
   - name: `Ada Lovelace`
   - email: `ada@example.com`
   - rating: `5`
   - comments: `Great product, loving it!`
   - subscribe: leave checked

7. **Click "Send feedback".** The form should submit as a new user chat message.

8. **Wait for the agent's reply.** It should acknowledge the submission and echo the field values back.

## Expected Result

### Capability Wiring

- The `a2ui` capability is registered on the agent and contributes its system-prompt addition (catalog + rules + action types).
- The system prompt sent to the LLM includes the line ` ```a2ui ` and the signatures for `Form`, `TextField`, `Textarea`, `Select`, `Checkbox`, and `Button`.
- No OpenUI prompt is included for this agent.

### Turn 1 — Form Emission & Render

- The raw assistant message contains a single ` ```a2ui `…` ``` ` fenced block.
- Inside the block, the JSON parses to a `Form` with `name: "feedback"` (or similar) and five children in the requested order.
- The chat UI **does not** show the raw JSON — it shows a styled form with:
  - a `<label>Your name</label>` + single-line input (required)
  - a `<label>Email</label>` + single-line input
  - a `<label>Rating</label>` + native `<select>` with options 1..5
  - a `<label>Comments</label>` + multi-line textarea with ~4 rows
  - a checkbox labelled "Send me product updates", pre-checked
  - a submit `<Button>` labelled **Send feedback**
- The form is wrapped in the A2UI block container (border + padding).
- No error-boundary fallback is rendered.

### Turn 2 — Submission

- Clicking **Send feedback** posts a new user message into the session. The message body either matches the agent's `submitMessage` prop or, if the prop was omitted, follows the default shape `Submitted {name} form: name="Ada Lovelace", email="ada@example.com", rating="5", comments="Great product, loving it!", subscribe=true`.
- The agent receives the submission as a normal chat turn and replies with a summary that includes the collected values.

### Security

- In the raw emitted JSON, any `Button.action.type === "open_url"` with `url` starting `javascript:` or `data:` is rejected by the renderer (`isSafeUrl`). This is out of scope for the happy path but worth checking by asking the LLM to add an "Open homepage" button in a follow-up turn and confirming only `http://`/`https://`/`mailto:` URLs open a new tab.

### Failure Modes

| Failure | What to look for |
|---------|-----------------|
| Capability not registered | Agent cannot be saved with `a2ui` enabled, or the system prompt lacks the `## Catalog` section |
| Raw JSON visible in chat | The ```a2ui block is rendered as a markdown code block instead of a form — check `splitA2UIBlocks` and `message-content.tsx` dispatch |
| Form renders but unstyled | shadcn primitives missing from imports in `a2ui-renderer.tsx` |
| Partial render during streaming freezes | The `…` placeholder never resolves after the stream ends — `isStreaming` not threaded through to `A2UIRoot` |
| Submit button inert | `FormContext` not providing `set`, or the submit handler doesn't call `dispatch({ type: "message", text })` |
| Empty submission message | `FormContext` collected values but `defaultSubmitMessage` returned a trivial string — field state may not be updating |
| Checkbox toggles nothing | `CheckboxNode` not wired to `useFieldValue` |
| `javascript:` URL opens | `isSafeUrl` not called before `window.open` — SECURITY regression |

## Notes

- Deterministic LLM output is not guaranteed; the model may add extra fields, reorder, or emit a slightly different JSON shape. Check rendered semantics (fields present, labels match, submission round-trips) rather than exact JSON.
- Run the test on at least one OpenAI and one Anthropic model; catalog-in-prompt capabilities behave differently per provider.
- If the LLM emits both ```openui and ```a2ui blocks, that indicates a prompt-ordering issue — only one generative-UI capability should be enabled per agent.
- The A2UI capability ships the `message` and `open_url` action types only; full server-side form round-trips (typed responses via tool call) are a v2 non-goal (see `specs/a2ui.md`).
