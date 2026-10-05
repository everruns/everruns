---
title: Computer Use
description: Let an agent see a browser through screenshots and operate it with clicks, typing, scrolling, and key presses, with any model that accepts images.
appliesTo: [platform]
---

| | |
|---|---|
| **ID** | `computer_use` |
| **Category** | Browser |
| **Features** | Leased resources |
| **Dependencies** | `session_storage` |
| **Risk** | High (the model sees and acts on whatever the screen shows) |
| **Availability** | Experimental |

Computer use lets an agent work a screen the way a person does. It takes a
screenshot, decides where to click or what to type, and gets a new screenshot
after each action. It works on pages that selector-based browser tools cannot
handle, such as canvas apps, heavy single-page apps, and custom widgets.

The capability is provider-neutral. It is a regular function tool that returns
images, so it works with OpenAI, Anthropic, Gemini, and any other model that
accepts images in tool results. On models with a native computer tool (OpenAI
GPT-5.4 and later through the Responses API, and recent Claude models with
`computer_toolset_20260801`), the agent uses the provider's own tool instead,
which those models are trained on. The display, the actions, and the limits
are the same either way.

The display is a Chromium page on a [Browserless](/capabilities/browserless/)
browser. Connect Browserless from **Settings → My agent experience** first.
The agent shares one persistent browser per session with the Browserless tools,
so a page opened by `browserless_open_browser` is the page `computer` sees.

## Tools

### `computer`

Performs one action and returns a screenshot of the display afterwards.
Coordinates are pixels in the latest screenshot, with the origin at the top
left.

| `action` | Parameters | What it does |
|---|---|---|
| `screenshot` | none | Capture the display |
| `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click` | `coordinate` (optional `[x, y]`, defaults to the cursor), `text` (optional modifiers such as `shift` or `ctrl+shift`) | Click |
| `left_click_drag` | `start_coordinate`, `coordinate` | Press, drag, and release |
| `mouse_move` | `coordinate` | Hover without clicking |
| `scroll` | `scroll_direction` (`up`, `down`, `left`, `right`), `scroll_amount` (wheel clicks, 1 to 50), optional `coordinate` | Scroll |
| `type` | `text` | Type text at the keyboard focus |
| `key` | `text` (a key or combo such as `Return`, `Tab`, `ctrl+a`), optional `repeat` | Press keys |
| `wait` | `duration` (seconds, up to 30) | Pause |
| `navigate` | `url` | Load a page |

Example call:

```json
{ "action": "left_click", "coordinate": [412, 230] }
```

## Configuration

| Field | Default | Description |
|---|---|---|
| `display_width` | `1280` | Display width in pixels (320 to 1920) |
| `display_height` | `800` | Display height in pixels (320 to 1200) |
| `screenshot_after_action` | `true` | Return a screenshot after every action. When off, only `screenshot` returns an image. |
| `max_actions_per_session` | `300` | Hard cap on actions in one session, screenshots included |
| `native_tools` | `true` | Use the provider's native computer tool on models that have one. When off, every model uses the `computer` function tool. |

Screenshots are billed as image tokens. A smaller display, or turning off
`screenshot_after_action`, lowers the cost per step.

## Safety

- **The model sees everything on the screen.** Text on a page can try to steer
  the agent. The capability tells the model to treat screen contents as
  untrusted and to stop and ask before typing credentials, making purchases,
  sending messages, or confirming irreversible actions. Add the
  `soft_approval` capability when you want those confirmations recorded, and
  the [`tool_approval`](/capabilities/tool-approval/) capability when a person
  must approve every `computer` call before it runs.
- **Approval before committing input.** In hosted sessions, a `computer` call
  that types text, presses Enter, or navigates waits for a person to approve
  that exact call, and so does a call the provider flags with a safety check.
  The request appears in the session like any other tool approval. Clicks,
  scrolling, and screenshots run without asking.
- **Keep credentials out of reach.** Do not give a computer-use agent a browser
  that is signed in to accounts it should not use.
- **Egress.** `navigate` refuses private and internal addresses and follows the
  session's network access list. Everruns makes every request the page makes and
  applies the same rules to each one, including redirects and DNS answers. If a
  page navigates somewhere blocked on its own, the page is reset to a blank page
  and the action reports an error.
- **Budget.** `max_actions_per_session` stops runaway loops.

Each screenshot shows as a thumbnail on the tool call in the session view;
click it for the full frame.

## Desktop display (experimental)

The `computer_use_desktop` capability gives the same `computer` tool a full
Linux desktop instead of a browser page, so the agent can work in any desktop
app. The desktop runs in an [E2B](/capabilities/e2b/) sandbox built from E2B's
`desktop` template, which the session creates on first use, keeps between
calls, and deletes when the session's lease ends. Connect E2B first. It takes
the same configuration as `computer_use`.

- There is no `navigate` action: the agent opens a browser on the desktop and
  types the address.
- The desktop has the E2B sandbox's network access, the same as
  `e2b_create_sandbox`. The session's network access list does not apply
  inside it.
- Enable either `computer_use` or `computer_use_desktop` on an agent, not both:
  they share the `computer` tool.
