---
type: Specification
title: "Markdown Messages Rendering"
description: "How the console renders agent and user Markdown: the Words renderer, its hooks and slots, and why it replaced Streamdown."
tags:
  - everruns
  - ui
---
# Markdown Messages Rendering

## Abstract

Every place the console shows Markdown written by agents or people (chat replies while they
stream and after, descriptions, prompts, file previews, traces) goes through one component,
[`MarkdownMessage`](../../apps/ui/src/components/markdown/markdown-message.tsx). It renders
with Words (`@pierre/words`, Apache-2.0), a streaming GFM renderer written in WebAssembly.

## Requirements

1. **Streaming is cheap.** A long reply re-renders on every token batch. The renderer must
   keep that well inside a frame on a mid-range phone, and must not grow memory with the
   length of the session.
2. **Unterminated syntax renders sensibly** while a reply streams: open code fences, half
   links, unclosed emphasis.
3. **GFM**: tables, task lists, strikethrough, footnotes, autolinks.
4. **Console additions on top of GFM**:
   - GitHub alerts (`> [!NOTE]` and the other four kinds) render as styled callouts.
   - Links open in a new tab, carry a source icon (Everruns page icon, GitHub, X, web), and
     unsafe destinations (`javascript:`, `data:` and the like) render as plain text.
     In-page anchors such as footnote links stay in the page.
   - `#cite-<n>` markers become citation chips when the message has citations.
   - Code blocks are highlighted in the console's light and dark themes.
   - Mermaid fences render as diagrams that follow the theme.
5. **No raw HTML** from the text is rendered.
6. **Pay for what is used**: the highlighter and the diagram renderer load only when a
   message contains a code fence or a Mermaid fence.

## Why Words

Measured on 2026-11-18 against the Streamdown setup it replaced, in Chromium, streaming real
repository Markdown 24 characters at a time (median of 3 runs):

| Message | Streamdown | Words |
|---|---|---|
| 30 KB reply, 1,255 steps | 2,263 ms CPU | 638 ms CPU |
| same, CPU slowed 4x | 11,971 ms | 4,329 ms |
| JS heap after GC, same reply | 23.8 MB | 3.0 MB |
| Reopening a session with 30 such replies | 1,823 ms | 582 ms |

Per streamed step on the long reply that is about 1.8 ms against 0.5 ms; with a slowed CPU,
9.5 ms (more than half a frame) against 3.4 ms. Output parity on the same reply: identical
tables, highlighted code blocks and headings. Downloaded size up front is similar (about
350 KB gzip including React), and Words carries every grammar in that, where Shiki fetched
grammars on demand.

Streamdown is still a dependency only because the legacy A2UI renderer in `generative-ui/`
uses it.

## Design

- **Hooks, not plugins.** Words has no remark/rehype pipeline. It hands each generated
  element to a hook as an HTML string and takes a string back.
  [`markdown-hooks.ts`](../../apps/ui/src/components/markdown/markdown-hooks.ts) holds the
  alert and link hooks as pure string transforms, unit-tested without a DOM.
- **Slots for React.** Words owns the DOM under its container and rewrites the streaming
  tail with `innerHTML`, so React cannot render inside it. Hooks leave empty
  `data-md-slot` placeholders; [`MarkdownSlots`](../../apps/ui/src/components/markdown/markdown-slots.tsx)
  watches the container and portals the link icon or citation chip into each one.
- **Highlighting** uses `@pierre/highlights` with GitHub light and dark themes and
  `light-dark()` colors, so it follows `color-scheme`. The tabbed
  [`CodeBlock`](../../apps/ui/src/components/ui/code-block.tsx) uses the same highlighter.
- **Diagrams** use Pele through Words' Mermaid plugin, with links and images off because
  diagrams are agent-written. Pele's palette variables map to design tokens in
  [`markdown-message.css`](../../apps/ui/src/components/markdown/markdown-message.css).
- **Raw HTML**: Words 0.2.x always omits it. `skipHtml` callers (work logs, traces, package
  review) mark the places that must stay HTML-free if a later Words version allows a safe
  tag allowlist elsewhere.

## Rendering contexts

| Content | Use |
|---|---|
| Streaming agent reply | `MarkdownMessage isAnimating` |
| Finished agent reply, static Markdown | `MarkdownMessage` |
| Descriptions and one-line text | `InlineMarkdownMessage` (no highlighting) |
| File previews | `MarkdownMessage variant="compact"` |
| User messages | plain text, whitespace kept |

## Transcript surfaces

- Message Markdown, tool activity and todo progress read as one transcript system. Tool rows
  stay inline with the surrounding message rhythm; do not nest bordered tool or todo cards
  inside another transcript card unless the content needs its own viewport.
- Platform Chat and Chats fold multi-iteration or tool work into a compact
  "Working / Worked for ..." affordance; a direct one-iteration answer needs no work log.
  Playground (testing) and Sessions (debugging) show every loaded work entry inline in
  transcript order, without turn folds or pagination.
- Work logs may show safe narration from reason summaries, tool headlines and
  commentary-phase assistant text. Raw hidden thinking stays out of the transcript. A
  text-only message whose phase was only inferred stays a normal answer.

## Styling

Code blocks use sharp corners and the sidebar surface, links use `--primary`, and every color
comes from design tokens so light and dark themes need no separate rules.
