---
type: Specification
title: "A2UI, Google Generative UI Integration"
description: "A2UI generative-UI capability."
tags:
  - everruns
  - ui
---
# A2UI, Google Generative UI Integration

## Purpose

Parallel generative-UI capability alongside [OpenUI](openui.md), built on Google's
**A2UI (Agent-to-User Interface)** open protocol. A2UI lets the agent describe UI
*intent* as declarative JSON; the client renders it using its own design system and
component library.

Two capabilities coexist:

| Capability | DSL                       | Renderer library        | Strength                               |
| ---------- | ------------------------- | ----------------------- | -------------------------------------- |
| `openui`   | OpenUI Lang (custom DSL)  | `@openuidev/react-ui`   | ~55 prebuilt React components          |
| `a2ui`     | JSON component tree       | Everruns shadcn/ui      | Design-system native, framework-neutral |

They are independently opt-in per agent. Nothing switches; agents select whichever
they prefer, or run neither.

## Decisions

- **Prompt-first, not strict schema.** Follow A2UI v0.9, embed the catalog in the
  system prompt and let the model emit JSON freely. We validate lightly at render
  time and render partial trees rather than rejecting on the first error.
- **JSON over DSL.** A2UI is JSON; parsers exist in every language. This is A2UI's
  key portability win over OpenUI Lang.
- **Fenced code block transport.** Agent emits `` ```a2ui `` fenced blocks inline
  with markdown, mirroring the OpenUI pattern. No separate wire protocol needed,
  our existing SSE message stream is enough.
- **Native renderer.** The UI renders A2UI JSON using Everruns' own shadcn/ui
  components. No third-party npm dependency. This is A2UI's model: the client owns
  the design system.
- **Small canonical catalog.** Ship the smallest useful cross-framework component
  set. The spec explicitly encourages custom catalogs; teams can extend later.
- **No bidirectional messaging yet.** A2UI v0.9 defines client→server updates for
  interactive surface state. Our v1 is read-only (buttons that emit chat messages).
  Form round-trips land in a follow-up.

## Architecture

```
┌──────────────────┐    ┌───────────────────┐    ┌────────────────────────┐
│  a2ui crate      │───▶│ a2ui capability   │───▶│   LLM system prompt    │
│ (catalog +       │    │ (everruns-core)   │    │  with A2UI catalog +   │
│  prompt gen)     │    │                   │    │  JSON emission rules   │
└──────────────────┘    └───────────────────┘    └────────────────────────┘
                                                           │
                                                           ▼
                                                 ┌───────────────────┐
                                                 │  LLM response     │
                                                 │  with ```a2ui     │
                                                 │  JSON blocks      │
                                                 └─────────┬─────────┘
                                                           ▼
                                                 ┌───────────────────┐
                                                 │  Chat UI          │
                                                 │  MessageContent   │
                                                 │  splits blocks    │
                                                 └─────────┬─────────┘
                                                           ▼
                                                 ┌───────────────────┐
                                                 │  A2UIBlock        │
                                                 │  (JSON tree →     │
                                                 │   shadcn/ui)      │
                                                 └───────────────────┘
```

## Wire Format

The LLM emits one JSON object per block, fenced with `` ```a2ui ``:

````
```a2ui
{
  "type": "Card",
  "children": [
    { "type": "Heading", "props": { "text": "Monthly Revenue" } },
    { "type": "Text", "props": { "text": "Q1 totaled $158k" } },
    {
      "type": "ButtonGroup",
      "children": [
        { "type": "Button", "props": { "label": "Details", "action": { "type": "message", "text": "Show details" } } }
      ]
    }
  ]
}
```
````

Every node is `{ type, props?, children? }`. Props are per-component. Children is a
flat array of nested nodes. Streaming-friendly: partial trees render as soon as
they parse.

## Module: `everruns_builtins::a2ui`

Path: `crates/builtins/src/a2ui/`.

Mirrors the `everruns_builtins::openui` pattern: static Rust catalog definitions plus a
prompt generator. No runtime parsing, the LLM receives a prompt and the renderer
lives in the UI.

### Types

- `ComponentDef`, `{ name, props, description, has_children }`
- `PropDef`, `{ name, type_annotation, optional, description }`
- `ComponentCategory`, logical grouping for the prompt
- `Catalog`, root component hint, components, categories
- `PromptOptions`, custom preamble, additional rules, examples

### Prompt sections

`generate_prompt(catalog, options)` emits:

1. **Preamble**: when and how to use `` ```a2ui `` blocks
2. **Schema rules**: JSON shape, every node has `type`, optional `props`, optional `children`
3. **Catalog**: components grouped by category with prop signatures and descriptions
4. **Action types**: `{ type: "message", text }`, `{ type: "open_url", url }`
5. **Streaming guidance**: emit shell first, fill children progressively
6. **Important rules**: stay within catalog, omit unknown props, prefer lists over repeats

Ref: `crates/builtins/src/a2ui/prompt.rs`.

### Canonical catalog

Minimal v1 catalog; extend later. All components map to existing Everruns UI
primitives under `apps/ui/src/components/ui/`.

| Category   | Components                                                |
| ---------- | --------------------------------------------------------- |
| Layout     | `Stack`, `Card`, `Separator`                              |
| Content    | `Heading`, `Text`, `Callout`, `Badge`, `Image`, `CodeBlock` |
| Data       | `List`, `ListItem`, `Table`                               |
| Forms      | `Form`, `TextField`, `Textarea`, `Select`, `Checkbox`     |
| Actions    | `Button`, `ButtonGroup`                                   |

## Capability: `a2ui`

ID: `a2ui`. Feature: `a2ui`. Category: `UI`.

Registered by the hosted product's portable catalog when
`everruns-builtins/ui-capabilities` is enabled. The capability appends the A2UI
prompt to the agent's system prompt and contributes no tools.

The capability coexists with `openui`. Enabling both is legal but wasteful,
instruct the agent to prefer one. Neither is enabled by default.

Ref: `crates/builtins/src/a2ui.rs`.

## UI Integration

### Block detection

`apps/ui/src/lib/a2ui-utils.ts` splits message text into alternating markdown and
A2UI segments. Regex: `` /```a2ui\s*\n([\s\S]*?)(?:```|$)/g ``. Streaming-safe:
unclosed blocks at end of string render as partial JSON.

### Renderer

`apps/ui/src/components/chat/a2ui-renderer.tsx`:

- `<A2UIBlock code={json} isStreaming={bool} />`
- Parses JSON with fault-tolerance: returns a partial tree for truncated input
- Walks the tree, dispatching each node to a component map
- Component map maps `type` string → React component using shadcn/ui primitives
- Unknown types render a subtle placeholder (not a hard error)
- Button actions `{ type: "message", text }` send a new user message through the
  existing chat input
- Wrapped in an error boundary that falls back to rendering the raw JSON as a
  code block
- URLs in `Image.src` and `open_url` actions are restricted to
  `http:`/`https:`/`mailto:` schemes to block `javascript:` and `data:` XSS
  (see THREAT[TM-WEB-A2UI-01])

### Message content

`apps/ui/src/components/chat/message-content.tsx` handles **both** openui and
a2ui blocks. Fast path checks for either; the split walks both patterns.

## Example

User: "Show me a summary of last week's orders."

LLM response:

````
Here's the weekly summary:

```a2ui
{
  "type": "Card",
  "children": [
    { "type": "Heading", "props": { "text": "Orders, Week of Apr 7", "level": 3 } },
    {
      "type": "List",
      "children": [
        { "type": "ListItem", "props": { "title": "Shipped", "value": "247" } },
        { "type": "ListItem", "props": { "title": "Pending", "value": "18" } },
        { "type": "ListItem", "props": { "title": "Returned", "value": "4" } }
      ]
    },
    {
      "type": "ButtonGroup",
      "children": [
        { "type": "Button", "props": { "label": "See shipped", "action": { "type": "message", "text": "List shipped orders" } } },
        { "type": "Button", "props": { "label": "Handle returns", "variant": "secondary", "action": { "type": "message", "text": "Process pending returns" } } }
      ]
    }
  ]
}
```

Let me know which you want to drill into.
````

Renders a Card with a heading, a three-row summary list, and two action buttons.
Clicking a button sends the `action.text` as a new chat message.

## Non-goals (v1)

- Form submissions with typed responses (round-trip via tool call comes later)
- Client→server surface-state updates (A2UI bidirectional messaging)
- Catalog negotiation handshake (we ship a fixed catalog per capability)
- Non-React renderers (A2UI JSON is portable; adding native renderers is a future
  extension)

## References

- A2UI project: <https://github.com/google/a2ui>
- CopilotKit writeup on A2UI v0.9: <https://www.copilotkit.ai/blog/a2ui-whats-new-in-google-generative-ui-spec>
- Sibling spec: [`knowledge/ui/openui.md`](openui.md)
