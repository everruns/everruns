"use client";

import { useUiTranslations } from "@/providers/locale-provider";
import { useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ArrowLeft } from "lucide-react";
import { MarkdownMessage, InlineMarkdownMessage } from "@/components/markdown/markdown-message";
import { StreamingMessage } from "@/components/streaming-message";
import { usePageTitle } from "@/hooks";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

// ============================================
// Showcase Section Components
// ============================================

function ShowcaseSection({
  title,
  description,
  children,
}: {
  title: string;
  description?: string;
  children: React.ReactNode;
}) {
  return (
    <Card className="min-w-0">
      <CardHeader>
        <CardTitle className="text-lg">{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      <CardContent className="min-w-0 space-y-4">{children}</CardContent>
    </Card>
  );
}

function ShowcaseItem({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="min-w-0 space-y-2">
      <div className="text-sm font-medium text-muted-foreground">{label}</div>
      <div className="min-w-0 border bg-background p-4">{children}</div>
    </div>
  );
}

// ============================================
// Markdown Content Samples
// ============================================

const sampleMarkdownContent = {
  basicFormatting: `**Bold text**, *italic text*, and \`inline code\`.

Here's a [link to GitHub](https://github.com).`,

  codeBlock: `Here's a code example:

\`\`\`typescript
function greet(name: string): string {
  return \`Hello, \${name}!\`;
}
\`\`\``,

  markdownInMarkdown: `Here's markdown inside a code block:

\`\`\`markdown
# Documentation

This is **bold** and *italic* text.

- List item 1
- List item 2

| Col A | Col B |
|-------|-------|
| 1     | 2     |
\`\`\``,

  lists: `Ordered list:
1. First item
2. Second item
3. Third item

Unordered list:
- Item A
- Item B
- Item C`,

  table: `| Feature | Status | Notes |
|---------|--------|-------|
| Markdown | Done | Full GFM support |
| Streaming | Done | Via Words |
| Tables | Done | With styling |`,

  diagram: `\`\`\`mermaid
flowchart LR
  Request --> Agent --> Reply
\`\`\``,

  capabilityDescription: `Fetch content from URLs and convert HTML to markdown.

**Features:**
- HTTP/HTTPS support
- HTML to Markdown conversion
- Configurable timeouts`,

  alerts: `GitHub-style alerts:

> [!NOTE]
> Highlights information that users should take into account.

> [!TIP]
> Optional information to help a user be more successful.

> [!IMPORTANT]
> Crucial information necessary for users to succeed.

> [!WARNING]
> Critical content demanding immediate user attention due to potential risks.

> [!CAUTION]
> Negative potential consequences of an action.`,

  allowedHtml: `Press <kbd>Ctrl</kbd> + <kbd>C</kbd> to stop. Water is H<sub>2</sub>O, and x<sup>2</sup> grows fast.

<details>
<summary>Full test output (42 lines)</summary>

All 42 tests passed in **3.1 s**.

</details>

<img src="x" onerror="alert(1)"> <span style="color:red">Disallowed tags show only their text.</span>`,

  longStreaming: `# Agent Response

I'll help you implement a new feature for your application.

## Overview

This implementation involves several steps:

1. **Create the component** - We'll build a reusable React component
2. **Add state management** - Using hooks for local state
3. **Style with Tailwind** - Consistent with the design system

## Code Example

\`\`\`typescript
import { useState } from 'react';

interface FeatureProps {
  title: string;
  enabled?: boolean;
}

export function Feature({ title, enabled = true }: FeatureProps) {
  const [isActive, setIsActive] = useState(enabled);

  return (
    <div className="p-4 border">
      <h3 className="text-lg font-medium">{title}</h3>
      <button onClick={() => setIsActive(!isActive)}>
        {isActive ? 'Disable' : 'Enable'}
      </button>
    </div>
  );
}
\`\`\`

## Next Steps

- [ ] Test the component
- [ ] Add documentation
- [ ] Deploy to staging

Let me know if you need any modifications!`,
};

// ============================================
// Streaming Demo Component
// ============================================

function StreamingDemo() {
  const ui = useUiTranslations();
  const [streamedText, setStreamedText] = useState("");
  const [isStreaming, setIsStreaming] = useState(false);
  const fullText = sampleMarkdownContent.longStreaming;

  const startStreaming = () => {
    setStreamedText("");
    setIsStreaming(true);
    let index = 0;

    const interval = setInterval(() => {
      if (index < fullText.length) {
        // Stream 3-8 characters at a time to simulate token streaming
        const chunkSize = Math.floor(Math.random() * 6) + 3;
        setStreamedText(fullText.slice(0, index + chunkSize));
        index += chunkSize;
      } else {
        clearInterval(interval);
        setIsStreaming(false);
      }
    }, 50);

    return () => clearInterval(interval);
  };

  const reset = () => {
    setStreamedText("");
    setIsStreaming(false);
  };

  return (
    <div className="space-y-4">
      <div className="flex gap-2">
        <button
          onClick={startStreaming}
          disabled={isStreaming}
          className="px-4 py-2 text-sm font-medium bg-primary text-primary-foreground disabled:opacity-50"
        >
          {isStreaming ? ui("Streaming...") : ui("Start Streaming Demo")}
        </button>
        <button onClick={reset} className="px-4 py-2 text-sm font-medium border hover:bg-muted">
          {ui("Reset")}
        </button>
      </div>

      <div className="border p-4 min-h-[400px] bg-background">
        {streamedText ? (
          isStreaming ? (
            <StreamingMessage messageId="dev-markdown-stream" text={streamedText} />
          ) : (
            <MarkdownMessage variant="inline">{streamedText}</MarkdownMessage>
          )
        ) : (
          <p className="text-muted-foreground italic">
            {ui('Click "Start Streaming Demo" to see Words handle incomplete markdown')}
          </p>
        )}
      </div>
    </div>
  );
}

// ============================================
// Incomplete Markdown Demo
// ============================================

function IncompleteMarkdownDemo() {
  const incompleteExamples = [
    { label: "Incomplete bold", content: "This is **incomplete bold" },
    { label: "Incomplete italic", content: "This is *incomplete italic" },
    { label: "Incomplete code", content: "Here's `incomplete code" },
    { label: "Incomplete link", content: "Check [this link](https://example" },
    {
      label: "Incomplete code block",
      content: "```typescript\nfunction hello() {\n  console.log('hi')",
    },
    { label: "Incomplete heading", content: "## Heading in progress" },
  ];

  return (
    <div className="space-y-3">
      {incompleteExamples.map((example) => (
        <div key={example.label} className="flex min-w-0 flex-col gap-2 sm:flex-row sm:gap-4">
          <div className="text-sm text-muted-foreground sm:w-40 sm:shrink-0">{example.label}</div>
          <div className="min-w-0 flex-1 rounded border bg-muted/30 p-2">
            <MarkdownMessage variant="inline" isAnimating={true}>
              {example.content}
            </MarkdownMessage>
          </div>
        </div>
      ))}
    </div>
  );
}

// ============================================
// Main Page Component
// ============================================

export default function DevMarkdownPage() {
  const ui = useUiTranslations();
  usePageTitle(isDev ? "Markdown" : "Page not found", isDev ? "Dev" : null);
  // Show 404-like message in production
  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="text-muted-foreground mt-2">{ui("Page not found")}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="min-h-screen bg-muted/30">
      <div className="container mx-auto py-8 px-4">
        <div className="mb-8">
          <Link
            href="/dev"
            className="inline-flex items-center text-sm text-muted-foreground hover:text-foreground mb-4"
          >
            <ArrowLeft className="w-4 h-4 mr-2" />
            {ui("Back to Developer Tools")}
          </Link>
          <h1 className="text-3xl font-bold">{ui("Markdown Message Component")}</h1>
          <p className="text-muted-foreground mt-2">
            {ui("Streaming Markdown renderer using Words (WebAssembly)")}
          </p>
          <Badge variant="outline" className="mt-2">
            {ui("Development Mode")}
          </Badge>
        </div>

        <div className="space-y-8">
          {/* Streaming Demo - Featured */}
          <ShowcaseSection
            title={ui("Live Streaming Demo")}
            description={ui("Watch how Words handles incomplete markdown during streaming")}
          >
            <StreamingDemo />
          </ShowcaseSection>

          {/* Incomplete Markdown Handling */}
          <ShowcaseSection
            title={ui("Incomplete Markdown Handling")}
            description={ui("How Words handles unterminated markdown syntax")}
          >
            <IncompleteMarkdownDemo />
          </ShowcaseSection>

          {/* Basic Markdown Formatting */}
          <ShowcaseSection
            title={ui("Basic Formatting")}
            description={ui("Standard markdown formatting features")}
          >
            <ShowcaseItem label={ui("Text Formatting")}>
              <InlineMarkdownMessage>{sampleMarkdownContent.basicFormatting}</InlineMarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Code Block (with syntax highlighting)")}>
              <MarkdownMessage variant="inline">{sampleMarkdownContent.codeBlock}</MarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Markdown in Markdown (code block)")}>
              <MarkdownMessage variant="inline">
                {sampleMarkdownContent.markdownInMarkdown}
              </MarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Lists")}>
              <InlineMarkdownMessage>{sampleMarkdownContent.lists}</InlineMarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Table")}>
              <InlineMarkdownMessage>{sampleMarkdownContent.table}</InlineMarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="Mermaid diagram">
              <MarkdownMessage variant="inline">{sampleMarkdownContent.diagram}</MarkdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* GitHub Alerts */}
          <ShowcaseSection
            title={ui("GitHub-Style Alerts")}
            description={ui("NOTE, TIP, IMPORTANT, WARNING, CAUTION callouts")}
          >
            <ShowcaseItem label={ui("All Alert Types")}>
              <MarkdownMessage variant="inline">{sampleMarkdownContent.alerts}</MarkdownMessage>
            </ShowcaseItem>
            <ShowcaseItem label={ui("Allowed HTML")}>
              <MarkdownMessage variant="inline">
                {sampleMarkdownContent.allowedHtml}
              </MarkdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Component Variants */}
          <ShowcaseSection
            title={ui("Component Variants")}
            description={ui("Different display variants for various contexts")}
          >
            <ShowcaseItem label={ui("Default Variant (with background)")}>
              <MarkdownMessage>{sampleMarkdownContent.capabilityDescription}</MarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Compact Variant (no background)")}>
              <MarkdownMessage variant="compact">
                {sampleMarkdownContent.capabilityDescription}
              </MarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Inline Variant (for descriptions)")}>
              <InlineMarkdownMessage className="text-muted-foreground">
                {sampleMarkdownContent.capabilityDescription}
              </InlineMarkdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* isAnimating States */}
          <ShowcaseSection
            title={ui("Animation States")}
            description={ui("Compare isAnimating=true vs isAnimating=false")}
          >
            <ShowcaseItem label={ui("isAnimating={false} (default - completed message)")}>
              <MarkdownMessage variant="inline" isAnimating={false}>
                {sampleMarkdownContent.codeBlock}
              </MarkdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label={ui("isAnimating={true} (streaming message)")}>
              <MarkdownMessage variant="inline" isAnimating={true}>
                {sampleMarkdownContent.codeBlock}
              </MarkdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Usage Examples */}
          <ShowcaseSection
            title={ui("Usage in Code")}
            description={ui("How to use the MarkdownMessage components")}
          >
            <ShowcaseItem label={ui("Import")}>
              <pre className="bg-muted p-4 text-sm overflow-x-auto">
                {`import { MarkdownMessage, InlineMarkdownMessage } from "@/components/markdown/markdown-message";`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Streaming Message")}>
              <pre className="bg-muted p-4 text-sm overflow-x-auto">
                {`<MarkdownMessage isAnimating={true}>
  {streamingText}
</MarkdownMessage>`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Completed Message")}>
              <pre className="bg-muted p-4 text-sm overflow-x-auto">
                {`<MarkdownMessage isAnimating={false}>
  {messageContent}
</MarkdownMessage>`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label={ui("Inline for Descriptions")}>
              <pre className="bg-muted p-4 text-sm overflow-x-auto">
                {`<InlineMarkdownMessage className="text-muted-foreground">
  {description}
</InlineMarkdownMessage>`}
              </pre>
            </ShowcaseItem>
          </ShowcaseSection>
        </div>
      </div>
    </div>
  );
}
