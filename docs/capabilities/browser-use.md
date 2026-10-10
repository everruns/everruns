---
title: Browser Use
description: Let an agent work in a browser through the page structure, with element refs, form filling, and tabs, on any model.
appliesTo: [platform]
---

| | |
|---|---|
| **ID** | `browser_use` |
| **Category** | Browser |
| **Features** | Leased resources |
| **Dependencies** | `session_storage` |
| **Risk** | High (the model reads and acts on whatever the page shows) |
| **Availability** | Experimental |

Browser use lets an agent work in a web browser through what the page is made
of, not only what it looks like. The agent reads the page as an accessibility
tree, where every element it can act on carries a ref such as `ref_3`. It
then clicks, hovers, scrolls to, or fills that element by ref. It can still
take screenshots and act on viewport coordinates when the structure is not
enough, and it can open, switch, and close tabs.

The `browser` tool is a regular function tool, so it works with any model.
Screenshots and `zoom` need a model that accepts images; the other actions
return text. It follows the action vocabulary of Claude's
`browser_toolset_20260801`. On Claude models that take that toolset, the agent
uses Claude's own browser tool instead, which those models are trained on, and
reads the open tabs from the toolset's browser state. The actions, the safety
rules, and the limits are the same either way.

The browser is the session's persistent [Browserless](/capabilities/browserless/)
browser. Connect Browserless from **Settings → My agent experience** first.
The agent shares that browser with the Browserless tools and with
[Computer Use](/capabilities/computer-use/), so a page one opens is a page the
others see.

## Tools

### `browser`

Performs one action. Every successful result carries a `browser_state` object:
every open tab with its `tab_id`, title and URL, which tab is active, and in
`state_changes` the tabs opened since the previous result.

A target is either `{"type": "ref", "ref": "ref_3"}` or
`{"type": "coordinate", "x": 412, "y": 230}`, in pixels of the viewport.

| `action` | Parameters | What it does |
|---|---|---|
| `read_page` | optional `filter` (`interactive` or `all`), `depth`, `ref` | Read the page as an accessibility tree with refs. By default it lists what is in the viewport; `interactive` lists only the controls; `all` includes the rest of the page; `ref` reads one element's subtree |
| `find` | `query` | Up to 20 elements that best match a description such as `search box` or `sign up button` |
| `get_page_text` | none | The page's text, main content first |
| `form_input` | `target` (a ref), `value` (text, a choice, or `true`/`false` for a checkbox) | Set a field's value and fire its `input` and `change` events |
| `left_click`, `right_click`, `middle_click`, `double_click`, `triple_click` | `target`, optional `modifiers` such as `shift` | Click the element's center or the point |
| `hover`, `mouse_move` | `target` (`mouse_move` takes a coordinate) | Move the pointer |
| `left_click_drag` | `from`, `target` | Press, drag, and release |
| `left_mouse_down`, `left_mouse_up` | `target` | Press or release the left button there |
| `scroll` | `target`, `scroll_direction`, optional `scroll_amount` (1 to 10, default 3) | Scroll at that point |
| `scroll_to` | `target` (a ref) | Scroll the element into view |
| `type` | `text` | Type at the keyboard focus |
| `key` | `text` (space-separated keys or combos, such as `Tab Tab Enter` or `ctrl+a`), optional `repeat` | Press keys |
| `hold_key` | `text`, `duration` (seconds, up to 30) | Hold keys down, then release them |
| `wait` | `duration` (seconds, up to 30) | Pause |
| `screenshot` | none | Capture the viewport |
| `zoom` | `region` (`[x0, y0, x1, y1]`) | That part of the viewport, enlarged to the viewport size |
| `navigate` | `url`, or `back`, `forward`, `reload` | Load a page or step through history |
| `new_tab`, `list_tabs` | none | Open a blank tab and make it current, or list the tabs |
| `switch_tab`, `close_tab` | `tab_id` | Make a tab current, or close it |

Any action also takes an optional `tab_id` to act on that tab; it becomes the
current tab.

Example calls:

```json
{ "action": "find", "query": "email field" }
{ "action": "form_input", "target": { "type": "ref", "ref": "ref_4" }, "value": "ada@example.com" }
{ "action": "left_click", "target": { "type": "ref", "ref": "ref_7" } }
```

Refs belong to the page they were read from. After a navigation, or when the
page removes the element, using one returns an error that tells the agent to
read the page again; a ref is never applied to a different element.

`javascript_exec`, `file_upload`, `read_console`, and `read_network` are not
offered. A call that asks for one is refused.

## Configuration

| Field | Default | Description |
|---|---|---|
| `viewport_width` | `1280` | Viewport width in pixels (320 to 1920) |
| `viewport_height` | `800` | Viewport height in pixels (320 to 1200) |
| `max_actions_per_session` | `500` | Hard cap on actions in one session |
| `native_tools` | `true` | Use Claude's native browser toolset on models that have one. When off, every model uses the `browser` function tool. |

When Claude sends several browser actions in one turn and one fails, the
later ones are not run; each answers that an earlier action failed.

## Safety

- **The model reads everything on the page.** Text, tab titles, and URLs can
  try to steer the agent. The capability tells the model to treat them as
  untrusted data and to stop and ask before typing credentials, making
  purchases, sending messages, or confirming irreversible actions. Add
  [`tool_approval`](/capabilities/tool-approval/) when a person must approve
  every `browser` call before it runs.
- **Refs stay out of the page.** Everruns keeps the table that maps refs to
  elements; the page cannot read or move it, so a ref read as "Cancel" cannot
  be pointed at "Buy".
- **Page scripts cannot tamper with reads and fills.** `get_page_text` and
  `form_input` run in a separate script world, so a page that overrides
  built-in functions does not change what they read or write.
- **Keep credentials out of reach.** Do not give a browser-use agent a browser
  that is signed in to accounts it should not use.
- **Egress.** `navigate` takes only http and https addresses; an address
  without a scheme is read as https. It refuses private and internal addresses
  and follows the session's network access list. Everruns makes every request
  the page makes, in every tab, and applies the same rules to each one. If a
  page navigates somewhere blocked on its own, it is reset to a blank page and
  the action reports an error.
- **Budget.** `max_actions_per_session` stops runaway loops.

## Availability

Browser use is experimental and ships behind the `browserless_browser_use`
feature flag (`FEATURE_BROWSERLESS_BROWSER_USE` sets its rollout grade).
