---
title: Computer Use
description: Let an agent see a browser through screenshots and operate it with clicks, typing, scrolling, and key presses, with any model that accepts images.
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
accepts images in tool results.

The display is a Chromium page on a [Browserless](/capabilities/browserless/)
browser. Connect Browserless in **Settings > Connections > Browserless** first.
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
- **Keep credentials out of reach.** Do not give a computer-use agent a browser
  that is signed in to accounts it should not use.
- **Egress.** `navigate` refuses private and internal addresses and follows the
  session's network access list. If a page navigates somewhere blocked on its
  own, the page is reset to a blank page and the action reports an error.
- **Budget.** `max_actions_per_session` stops runaway loops.

Native OpenAI and Anthropic computer tools, and desktop displays on sandboxes,
are planned.
