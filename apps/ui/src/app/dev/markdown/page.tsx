"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ArrowLeft } from "lucide-react";
import { Markdown, InlineMarkdown } from "@/components/ui/markdown";

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

function ShowcaseItem({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
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
| Alerts | Done | All 5 types |
| Tables | Done | With styling |`,

  noteAlert: `> [!NOTE]
> Highlights information that users should take into account, even when skimming.`,

  tipAlert: `> [!TIP]
> Optional information to help a user be more successful.`,

  importantAlert: `> [!IMPORTANT]
> Crucial information necessary for users to succeed.`,

  warningAlert: `> [!WARNING]
> Critical content demanding immediate user attention due to potential risks.`,

  cautionAlert: `> [!CAUTION]
> Negative potential consequences of an action.`,

  allAlerts: `> [!NOTE]
> This is a note with helpful information.

> [!TIP]
> This is a tip to help you succeed.

> [!IMPORTANT]
> This is important information you must know.

> [!WARNING]
> This is a warning about potential risks.

> [!CAUTION]
> This describes negative consequences of an action.`,

  capabilityDescription: `Fetch content from URLs and convert HTML to markdown.

> [!TIP]
> Use \`as_markdown: true\` for better readability when fetching HTML pages.

> [!WARNING]
> Binary content (images, PDFs) cannot be fetched as text. Only metadata is returned.

**Features:**
- HTTP/HTTPS support
- HTML to Markdown conversion
- Configurable timeouts`,
};

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
          <h1 className="text-3xl font-bold">Markdown Component</h1>
          <p className="text-muted-foreground mt-2">
            GitHub Flavored Markdown renderer with alert support
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <div className="space-y-8">
          {/* Basic Markdown Formatting */}
          <ShowcaseSection
            title="Basic Formatting"
            description="Standard markdown formatting features"
          >
            <ShowcaseItem label="Text Formatting">
              <InlineMarkdown content={sampleMarkdownContent.basicFormatting} />
            </ShowcaseItem>

            <ShowcaseItem label="Code Block">
              <InlineMarkdown content={sampleMarkdownContent.codeBlock} />
            </ShowcaseItem>

            <ShowcaseItem label="Lists">
              <InlineMarkdown content={sampleMarkdownContent.lists} />
            </ShowcaseItem>

            <ShowcaseItem label="Table">
              <InlineMarkdown content={sampleMarkdownContent.table} />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* GitHub-Style Alerts Section */}
          <ShowcaseSection
            title="GitHub-Style Alerts"
            description="GitHub Flavored Markdown alerts: NOTE, TIP, IMPORTANT, WARNING, CAUTION"
          >
            <ShowcaseItem label="[!NOTE] Alert">
              <InlineMarkdown content={sampleMarkdownContent.noteAlert} />
            </ShowcaseItem>

            <ShowcaseItem label="[!TIP] Alert">
              <InlineMarkdown content={sampleMarkdownContent.tipAlert} />
            </ShowcaseItem>

            <ShowcaseItem label="[!IMPORTANT] Alert">
              <InlineMarkdown content={sampleMarkdownContent.importantAlert} />
            </ShowcaseItem>

            <ShowcaseItem label="[!WARNING] Alert">
              <InlineMarkdown content={sampleMarkdownContent.warningAlert} />
            </ShowcaseItem>

            <ShowcaseItem label="[!CAUTION] Alert">
              <InlineMarkdown content={sampleMarkdownContent.cautionAlert} />
            </ShowcaseItem>

            <ShowcaseItem label="All Alerts Together">
              <InlineMarkdown content={sampleMarkdownContent.allAlerts} />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Markdown Variants Section */}
          <ShowcaseSection
            title="Markdown Variants"
            description="Different display variants for various contexts"
          >
            <ShowcaseItem label="Default Variant (with background)">
              <Markdown content={sampleMarkdownContent.capabilityDescription} />
            </ShowcaseItem>

            <ShowcaseItem label="Compact Variant (no background)">
              <Markdown content={sampleMarkdownContent.capabilityDescription} variant="compact" />
            </ShowcaseItem>

            <ShowcaseItem label="InlineMarkdown (for descriptions)">
              <InlineMarkdown
                content={sampleMarkdownContent.capabilityDescription}
                className="text-muted-foreground"
              />
            </ShowcaseItem>
          </ShowcaseSection>

          {/* Usage Examples */}
          <ShowcaseSection
            title="Usage in Code"
            description="How to use the Markdown components"
          >
            <ShowcaseItem label="Import">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
{`import { Markdown, InlineMarkdown } from "@/components/ui/markdown";`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label="Full Markdown Block">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
{`<Markdown content={description} />
<Markdown content={description} variant="compact" />`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label="Inline Markdown (for descriptions)">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
{`<InlineMarkdown content={capability.description} className="text-muted-foreground" />`}
              </pre>
            </ShowcaseItem>

            <ShowcaseItem label="GitHub Alert Syntax">
              <pre className="bg-muted p-4 rounded-md text-sm overflow-x-auto">
{`> [!NOTE]
> Your note content here.

> [!TIP]
> Your tip content here.

> [!IMPORTANT]
> Your important content here.

> [!WARNING]
> Your warning content here.

> [!CAUTION]
> Your caution content here.`}
              </pre>
            </ShowcaseItem>
          </ShowcaseSection>
        </div>
      </div>
    </div>
  );
}
