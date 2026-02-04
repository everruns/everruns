# Markdown Messages Rendering

## Abstract

Specification for rendering markdown content in chat messages with streaming support. Uses `llm-ui` for optimized LLM output rendering with frame-rate synchronization and broken syntax handling.

## Requirements

### Library: llm-ui

**Package**: `llm-ui` ([llm-ui.com](https://llm-ui.com/))

**Why llm-ui**:
1. **Frame-rate sync** — Renders characters at display's native refresh rate, smoothing token-by-token streaming
2. **Pause elimination** — Smooths LLM response delays for seamless UX
3. **Broken syntax handling** — Removes incomplete markdown syntax during streaming
4. **Syntax highlighting** — Shiki integration with 100+ languages
5. **Custom blocks** — Extensible for tool results, buttons, etc.
6. **Model agnostic** — Works with any LLM (Claude, GPT, Ollama, etc.)

### Alternatives Considered

| Library | Why Not |
|---------|---------|
| **Streamdown** (Vercel) | Good drop-in for react-markdown, but less UX polish (no frame-rate sync, no pause elimination) |
| **Incremark** | Best raw performance (6-16x faster), but newer ecosystem; overkill for typical message lengths |
| **react-markdown** (current) | No streaming support; re-parses entire document on each token causing O(n²) perf |
| **AI SDK memoization** | DIY approach; more code to maintain |

### Installation

```bash
cd apps/ui
npm install llm-ui @llm-ui/markdown @llm-ui/code
```

### Dependencies

| Package | Purpose |
|---------|---------|
| `llm-ui` | Core streaming renderer |
| `@llm-ui/markdown` | Markdown block support |
| `@llm-ui/code` | Syntax highlighting via Shiki |

## Architecture

### Component Structure

```
components/
├── ui/
│   └── markdown.tsx           # Existing - keep for static markdown (descriptions, prompts)
└── chat/
    └── llm-message.tsx        # NEW - streaming message renderer using llm-ui
```

### Rendering Strategy

| Content Type | Component | Library |
|--------------|-----------|---------|
| Streaming agent messages | `LlmMessage` | llm-ui |
| Completed agent messages | `LlmMessage` | llm-ui (isStreamFinished=true) |
| User messages | Plain text | None (whitespace-pre-wrap) |
| Static markdown (descriptions) | `Markdown` | react-markdown (existing) |

### Integration Points

1. **StreamingMessage component** (`streaming-message.tsx`)
   - Replace plain text with `LlmMessage`
   - Pass `isStreamFinished={false}` during streaming

2. **Chat page** (`chat/page.tsx`)
   - Agent messages use `LlmMessage` with `isStreamFinished={true}`
   - User messages remain plain text

## Usage

### Basic Streaming Message

```tsx
import { LlmMessage } from "@/components/chat/llm-message";

<LlmMessage
  content={streamingText}
  isStreamFinished={false}
/>
```

### Completed Message

```tsx
<LlmMessage
  content={message.text}
  isStreamFinished={true}
/>
```

### Custom Blocks (Future)

llm-ui supports custom block syntax for rich content:

```
【{type:"tool_result",id:"abc123"}】
```

This can be used to embed tool results, buttons, or other interactive elements directly in message content.

## Styling

Must integrate with existing design system:
- Code blocks: Use brand colors, sharp corners (0px radius)
- Links: Navy color (`--primary`)
- Syntax highlighting: Theme compatible with light/dark mode

## Unification Note

The existing `Markdown` component (`components/ui/markdown.tsx`) using react-markdown is kept for:
- Agent/capability descriptions
- System prompt preview in editor
- Other static markdown content

This avoids unnecessary migration of non-streaming content while providing optimized streaming for chat messages.

## Future Considerations

- **Incremark migration**: If performance becomes an issue with very long messages, consider migrating to Incremark for its O(n) incremental parsing
- **Custom blocks**: Implement tool result rendering via llm-ui custom blocks instead of separate components
- **Math support**: Add `@llm-ui/math` if LaTeX rendering is needed
