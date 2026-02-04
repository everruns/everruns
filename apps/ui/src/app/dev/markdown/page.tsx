"use client";

import { useState } from "react";
import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ArrowLeft } from "lucide-react";
import {
  StreamdownMessage,
  InlineStreamdownMessage,
} from "@/components/chat/streamdown-message";
import { StreamingMessage } from "@/components/streaming-message";

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
    <Card>
      <CardHeader>
        <CardTitle className="text-lg">{title}</CardTitle>
        {description && <CardDescription>{description}</CardDescription>}
      </CardHeader>
      <CardContent className="space-y-4">{children}</CardContent>
    </Card>
  );
}

function ShowcaseItem({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="space-y-2">
      <div className="text-sm font-medium text-muted-foreground">{label}</div>
      <div className="border rounded-lg p-4 bg-background">{children}</div>
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
| Streaming | Done | Via Streamdown |
| Tables | Done | With styling |`,

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
    <div className="p-4 border rounded-lg">
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
          className="px-4 py-2 text-sm font-medium bg-primary text-primary-foreground rounded-md disabled:opacity-50"
        >
          {isStreaming ? "Streaming..." : "Start Streaming Demo"}
        </button>
        <button
          onClick={reset}
          className="px-4 py-2 text-sm font-medium border rounded-md hover:bg-muted"
        >
          Reset
        </button>
      </div>

      <div className="border rounded-lg p-4 min-h-[400px] bg-background">
        {streamedText ? (
          isStreaming ? (
            <StreamingMessage text={streamedText} />
          ) : (
            <StreamdownMessage variant="inline">{streamedText}</StreamdownMessage>
          )
        ) : (
          <p className="text-muted-foreground italic">
            Click &quot;Start Streaming Demo&quot; to see Streamdown handle incomplete markdown
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
        <div key={example.label} className="flex gap-4">
          <div className="w-40 text-sm text-muted-foreground shrink-0">{example.label}</div>
          <div className="flex-1 border rounded p-2 bg-muted/30">
            <StreamdownMessage variant="inline" isAnimating={true}>
              {example.content}
            </StreamdownMessage>
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
  // Show 404-like message in production
  if (!isDev) {
    return (
      <div className="flex items-center justify-center min-h-screen">
        <div className="text-center">
          <h1 className="text-4xl font-bold text-muted-foreground">404</h1>
          <p className="text-muted-foreground mt-2">Page not found</p>
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
            Back to Developer Tools
          </Link>
          <h1 className="text-3xl font-bold">Streamdown Message Component</h1>
          <p className="text-muted-foreground mt-2">
            Streaming-optimized markdown renderer using Vercel&apos;s Streamdown
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <div className="space-y-8">
          {/* Streaming Demo - Featured */}
          <ShowcaseSection
            title="Live Streaming Demo"
            description="Watch how Streamdown handles incomplete markdown during streaming"
          >
            <StreamingDemo />
          </ShowcaseSection>

          {/* Incomplete Markdown Handling */}
          <ShowcaseSection
            title="Incomplete Markdown Handling"
            description="How Streamdown gracefully handles unterminated markdown syntax"
          >
            <IncompleteMarkdownDemo />
          </ShowcaseSection>

          {/* Basic Markdown Formatting */}
          <ShowcaseSection
            title="Basic Formatting"
            description="Standard markdown formatting features"
          >
            <ShowcaseItem label="Text Formatting">
              <InlineStreamdownMessage>{sampleMarkdownContent.basicFormatting}</InlineStreamdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="Code Block (with Shiki highlighting)">
              <StreamdownMessage variant="inline">{sampleMarkdownContent.codeBlock}</StreamdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="Markdown in Markdown (code block)">
              <StreamdownMessage variant="inline">{sampleMarkdownContent.markdownInMarkdown}</StreamdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="Lists">
              <InlineStreamdownMessage>{sampleMarkdownContent.lists}</InlineStreamdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="Table">
              <InlineStreamdownMessage>{sampleMarkdownContent.table}</InlineStreamdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* GitHub Alerts */}
          <ShowcaseSection
            title="GitHub-Style Alerts"
            description="NOTE, TIP, IMPORTANT, WARNING, CAUTION callouts"
          >
            <ShowcaseItem label="All Alert Types">
              <StreamdownMessage variant="inline">{sampleMarkdownContent.alerts}</StreamdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Component Variants */}
          <ShowcaseSection
            title="Component Variants"
            description="Different display variants for various contexts"
          >
            <ShowcaseItem label="Default Variant (with background)">
              <StreamdownMessage>{sampleMarkdownContent.capabilityDescription}</StreamdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="Compact Variant (no background)">
              <StreamdownMessage variant="compact">
                {sampleMarkdownContent.capabilityDescription}
              </StreamdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="Inline Variant (for descriptions)">
              <InlineStreamdownMessage className="text-muted-foreground">
                {sampleMarkdownContent.capabilityDescription}
              </InlineStreamdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* isAnimating States */}
          <ShowcaseSection
            title="Animation States"
            description="Compare isAnimating=true vs isAnimating=false"
          >
            <ShowcaseItem label="isAnimating={false} (default - completed message)">
              <StreamdownMessage variant="inline" isAnimating={false}>
                {sampleMarkdownContent.codeBlock}
              </StreamdownMessage>
            </ShowcaseItem>

            <ShowcaseItem label="isAnimating={true} (streaming message)">
              <StreamdownMessage variant="inline" isAnimating={true}>
                {sampleMarkdownContent.codeBlock}
              </StreamdownMessage>
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Usage Examples */}
          <ShowcaseSection title="Usage in Code" description="How to use the StreamdownMessage components">
            <ShowcaseItem label="Import">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
                {`import { StreamdownMessage, InlineStreamdownMessage } from "@/components/chat/streamdown-message";`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label="Streaming Message">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
                {`<StreamdownMessage isAnimating={true}>
  {streamingText}
</StreamdownMessage>`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label="Completed Message">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
                {`<StreamdownMessage isAnimating={false}>
  {messageContent}
</StreamdownMessage>`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label="Inline for Descriptions">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
                {`<InlineStreamdownMessage className="text-muted-foreground">
  {description}
</InlineStreamdownMessage>`}
              </pre>
            </ShowcaseItem>
          </ShowcaseSection>
        </div>
      </div>
    </div>
  );
}
