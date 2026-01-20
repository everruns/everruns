"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ArrowLeft, Zap } from "lucide-react";
import type { TokenUsage } from "@/lib/api/types";

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
// Token Usage Display
// ============================================

// Helper function to format token counts in a compact way
function formatTokens(tokens: number): string {
  if (tokens >= 1000000) {
    return `${(tokens / 1000000).toFixed(1)}M`;
  }
  if (tokens >= 1000) {
    return `${(tokens / 1000).toFixed(1)}K`;
  }
  return tokens.toString();
}

// Helper function to calculate total tokens
function totalTokens(usage: TokenUsage): number {
  return usage.input_tokens + usage.output_tokens;
}

// Sample usage data
const sampleUsageData = {
  small: {
    input_tokens: 128,
    output_tokens: 45,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
  } satisfies TokenUsage,
  medium: {
    input_tokens: 15234,
    output_tokens: 8721,
    cache_read_tokens: 5000,
    cache_creation_tokens: 0,
  } satisfies TokenUsage,
  large: {
    input_tokens: 1250000,
    output_tokens: 875000,
    cache_read_tokens: 500000,
    cache_creation_tokens: 125000,
  } satisfies TokenUsage,
};

// Token Usage Card component (matches agent detail page)
function TokenUsageCard({ usage, title }: { usage: TokenUsage; title: string }) {
  return (
    <Card>
      <CardHeader className="pb-2">
        <CardTitle className="text-sm">{title}</CardTitle>
      </CardHeader>
      <CardContent>
        <div className="flex items-center gap-2 p-2 rounded-md border bg-muted/50">
          <Zap className="w-4 h-4 text-yellow-500" />
          <div className="flex-1">
            <p className="text-sm font-medium">{formatTokens(totalTokens(usage))} total</p>
            <p className="text-xs text-muted-foreground">
              {formatTokens(usage.input_tokens)} input / {formatTokens(usage.output_tokens)} output
              {usage.cache_read_tokens ? ` / ${formatTokens(usage.cache_read_tokens)} cached` : ""}
            </p>
          </div>
        </div>
      </CardContent>
    </Card>
  );
}

// Token Usage Badge component (matches session header)
function TokenUsageBadge({ usage }: { usage: TokenUsage }) {
  return (
    <Badge variant="outline" className="gap-1" title="Token usage (input/output)">
      <Zap className="w-3 h-3" />
      {formatTokens(usage.input_tokens)} / {formatTokens(usage.output_tokens)}
    </Badge>
  );
}

// ============================================
// Main Page Component
// ============================================

export default function DevSessionComponentsPage() {
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
          <h1 className="text-3xl font-bold">Session Components</h1>
          <p className="text-muted-foreground mt-2">
            Components for session-level UI: token usage, status badges, and headers
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <ScrollArea className="h-[calc(100vh-12rem)]">
          <div className="space-y-8 pr-4">
            {/* Token Usage Display Section */}
            <ShowcaseSection
              title="Token Usage Display"
              description="Components showing LLM token usage statistics for agents and sessions"
            >
              <ShowcaseItem label="Usage Card (Agent Detail Page)">
                <div className="grid grid-cols-3 gap-4">
                  <TokenUsageCard usage={sampleUsageData.small} title="Small Usage" />
                  <TokenUsageCard usage={sampleUsageData.medium} title="Medium Usage" />
                  <TokenUsageCard usage={sampleUsageData.large} title="Large Usage" />
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Usage Badge (Session Header)">
                <div className="flex items-center gap-4">
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-muted-foreground">Small:</span>
                    <TokenUsageBadge usage={sampleUsageData.small} />
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-muted-foreground">Medium:</span>
                    <TokenUsageBadge usage={sampleUsageData.medium} />
                  </div>
                  <div className="flex items-center gap-2">
                    <span className="text-sm text-muted-foreground">Large:</span>
                    <TokenUsageBadge usage={sampleUsageData.large} />
                  </div>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Session Header with Usage">
                <div className="border rounded-lg p-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <h2 className="text-lg font-bold">Example Session</h2>
                      <p className="text-sm text-muted-foreground">Started Jan 15, 2026, 10:30 AM</p>
                    </div>
                    <div className="flex items-center gap-2">
                      <TokenUsageBadge usage={sampleUsageData.medium} />
                      <Badge variant="outline" className="gap-1">claude-sonnet-4</Badge>
                      <Badge variant="secondary">Ready</Badge>
                    </div>
                  </div>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Status Badges Section */}
            <ShowcaseSection
              title="Status Badges"
              description="Session and agent status indicators"
            >
              <ShowcaseItem label="Session Status">
                <div className="flex items-center gap-4">
                  <Badge variant="secondary">pending</Badge>
                  <Badge variant="default" className="bg-blue-500">running</Badge>
                  <Badge variant="outline" className="border-green-500 text-green-600">completed</Badge>
                  <Badge variant="destructive">failed</Badge>
                  <Badge variant="outline" className="border-yellow-500 text-yellow-600">cancelled</Badge>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Agent Status">
                <div className="flex items-center gap-4">
                  <Badge variant="outline" className="border-green-500 text-green-600">active</Badge>
                  <Badge variant="secondary">inactive</Badge>
                  <Badge variant="destructive">error</Badge>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Model Badges">
                <div className="flex items-center gap-4">
                  <Badge variant="outline">claude-sonnet-4</Badge>
                  <Badge variant="outline">gpt-4o</Badge>
                  <Badge variant="outline">o1-preview</Badge>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Session Header Examples */}
            <ShowcaseSection
              title="Session Header Layouts"
              description="Different configurations of session headers"
            >
              <ShowcaseItem label="Minimal Header">
                <div className="border rounded-lg p-4">
                  <div className="flex items-center justify-between">
                    <h2 className="text-lg font-semibold">Session #1234</h2>
                    <Badge variant="secondary">pending</Badge>
                  </div>
                </div>
              </ShowcaseItem>

              <ShowcaseItem label="Full Header with All Metadata">
                <div className="border rounded-lg p-4">
                  <div className="flex items-center justify-between">
                    <div>
                      <h2 className="text-lg font-bold">Production Debug Session</h2>
                      <p className="text-sm text-muted-foreground">Created 2 hours ago by user@example.com</p>
                    </div>
                    <div className="flex items-center gap-2">
                      <TokenUsageBadge usage={sampleUsageData.large} />
                      <Badge variant="outline">o1-preview</Badge>
                      <Badge variant="default" className="bg-blue-500">running</Badge>
                    </div>
                  </div>
                  <div className="mt-3 pt-3 border-t flex items-center gap-4 text-sm text-muted-foreground">
                    <span>Turn: 15</span>
                    <span>Messages: 47</span>
                    <span>Duration: 1h 23m</span>
                  </div>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
