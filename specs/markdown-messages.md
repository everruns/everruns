# Markdown Messages Rendering

## Abstract

Specification for rendering markdown content in chat messages with streaming support. Uses Streamdown for optimized LLM output rendering with broken syntax handling and memoized re-rendering.

## Requirements

### Library: Streamdown

**Package**: `streamdown` ([streamdown.ai](https://streamdown.ai/), [GitHub](https://github.com/vercel/streamdown))

**Why Streamdown**:
1. **React 19 support** — Requires React 19.1.1+, compatible with project's React 19.2.4
2. **Drop-in replacement** — Same props API as react-markdown (remarkPlugins, rehypePlugins)
3. **Unterminated block parsing** — Handles incomplete markdown during streaming gracefully
4. **Memoized rendering** — Only re-renders changed portions of document
5. **Syntax highlighting** — Shiki-based with copy/download buttons
6. **Security hardening** — Built-in XSS protection via rehype-harden
7. **GFM support** — Tables, task lists, strikethrough built-in

### Alternatives Considered

| Library | Why Not |
|---------|---------|
| **llm-ui** | Unmaintained (2+ years), requires React 18, incompatible with React 19 |
| **Incremark** | Best raw performance (6-16x faster), but newer ecosystem; overkill for typical message lengths |
| **react-markdown** (current) | No streaming support; re-parses entire document on each token causing O(n²) perf |
| **AI SDK memoization** | DIY approach; more code to maintain |

### Installation

```bash
cd apps/ui
npm install streamdown @streamdown/code
```

### Dependencies

| Package | Purpose |
|---------|---------|
| `streamdown` | Core streaming markdown renderer |
| `@streamdown/code` | Syntax highlighting via Shiki |

Optional plugins (add when needed):
- `@streamdown/math` — KaTeX math rendering
- `@streamdown/mermaid` — Diagram rendering

## Architecture

### Component Structure

```
components/
├── ui/
│   └── markdown.tsx           # REMOVE - replaced by streamdown-message
└── chat/
    └── streamdown-message.tsx # NEW - unified markdown renderer using Streamdown
```

### Rendering Strategy

| Content Type | Component | Props |
|--------------|-----------|-------|
| Streaming agent messages | `StreamdownMessage` | `isAnimating={true}` |
| Completed agent messages | `StreamdownMessage` | `isAnimating={false}` |
| User messages | Plain text | None (whitespace-pre-wrap) |
| Static markdown (descriptions) | `StreamdownMessage` | `isAnimating={false}` |

### Integration Points

1. **StreamingMessage component** (`streaming-message.tsx`)
   - Replace plain text with `StreamdownMessage`
   - Pass `isAnimating={true}` during streaming

2. **Chat page** (`chat/page.tsx`)
   - Agent messages use `StreamdownMessage` with `isAnimating={false}`
   - User messages remain plain text

3. **Static markdown** (descriptions, prompts)
   - Migrate from `Markdown` to `StreamdownMessage`
   - Unified component for all markdown rendering

4. **Transcript tool/todo surfaces**
   - Message markdown, tool activity, and todo progress must read as one transcript system
   - Tool rows should stay inline with the surrounding message rhythm
   - Do not nest bordered tool/todo cards inside another transcript card unless the content requires a dedicated viewport

## Usage

### Basic Streaming Message

```tsx
import { StreamdownMessage } from "@/components/chat/streamdown-message";

<StreamdownMessage isAnimating={true}>
  {streamingText}
</StreamdownMessage>
```

### Completed Message

```tsx
<StreamdownMessage isAnimating={false}>
  {message.text}
</StreamdownMessage>
```

### With Code Highlighting

```tsx
import { StreamdownMessage } from "@/components/chat/streamdown-message";

<StreamdownMessage
  isAnimating={false}
  enableCodeHighlighting={true}
>
  {contentWithCode}
</StreamdownMessage>
```

## Styling

Must integrate with existing design system:
- Code blocks: Use brand colors, sharp corners (0px radius per brand spec)
- Links: Navy color (`--primary`)
- Syntax highlighting: Theme compatible with light/dark mode
- Tailwind integration via `@source` directive in globals.css
- Message-adjacent tool/todo UI should avoid double-wrapped boxes and favor spacing, dividers, and accent borders

### Tailwind Setup

Add to `globals.css`:
```css
@source "../node_modules/streamdown/dist/*.js";
```

## Unification

Streamdown replaces both:
1. The old `Markdown` component (react-markdown based)
2. Plain text message rendering

Single component for all markdown needs, with streaming support when needed.

## Future Considerations

- **Incremark migration**: If performance becomes an issue with very long messages, consider migrating to Incremark for its O(n) incremental parsing
- **Math support**: Add `@streamdown/math` for LaTeX equations when needed
- **Mermaid diagrams**: Add `@streamdown/mermaid` for diagram rendering
