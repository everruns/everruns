"use client";

import Link from "next/link";
import { Card, CardContent, CardDescription, CardHeader, CardTitle } from "@/components/ui/card";
import { Badge } from "@/components/ui/badge";
import { ScrollArea } from "@/components/ui/scroll-area";
import { ArrowLeft } from "lucide-react";
import { SessionCard } from "@/components/session/session-card";
import type { Session, LlmModelWithProvider } from "@/lib/api/types";

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
// Sample Data for SessionCard
// ============================================

const sampleSessions = {
  running: {
    id: "019234ab-cdef-7890-1234-567890abcdef",
    agent_id: "agent-123",
    title: "Working on implementing user authentication with OAuth 2.0 support for Google and GitHub providers",
    tags: ["auth", "oauth"],
    model_id: "model-uuid-anthropic",
    status: "active" as const,
    created_at: new Date(Date.now() - 300000).toISOString(), // 5 minutes ago
    started_at: new Date(Date.now() - 240000).toISOString(), // 4 minutes ago
    finished_at: null,
  } satisfies Session,
  idle: {
    id: "019234cd-efgh-7890-1234-567890abcdef",
    agent_id: "agent-123",
    title: "Completed code review for the API endpoints",
    tags: ["review"],
    model_id: "model-uuid-openai",
    status: "idle" as const,
    created_at: new Date(Date.now() - 3600000).toISOString(), // 1 hour ago
    started_at: new Date(Date.now() - 3500000).toISOString(),
    finished_at: null,
  } satisfies Session,
  new: {
    id: "019234ef-ijkl-7890-1234-567890abcdef",
    agent_id: "agent-123",
    title: null,
    tags: [],
    model_id: null,
    status: "started" as const,
    created_at: new Date().toISOString(),
    started_at: null,
    finished_at: null,
  } satisfies Session,
  longSummary: {
    id: "019234gh-mnop-7890-1234-567890abcdef",
    agent_id: "agent-123",
    title: "Investigating the performance bottleneck in the database query layer. Found that the N+1 query problem is causing significant slowdowns when fetching related entities. Working on implementing eager loading with batch fetching to reduce the number of database round trips.",
    tags: ["performance", "database", "optimization"],
    model_id: "model-uuid-anthropic",
    status: "active" as const,
    created_at: new Date(Date.now() - 1800000).toISOString(), // 30 minutes ago
    started_at: new Date(Date.now() - 1700000).toISOString(),
    finished_at: null,
  } satisfies Session,
};

const sampleModels = {
  anthropic: {
    id: "model-uuid-anthropic",
    provider_id: "provider-anthropic",
    model_id: "claude-sonnet-4-20250514",
    display_name: "Claude Sonnet 4",
    capabilities: ["chat", "tools"],
    is_default: true,
    status: "active" as const,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    provider_name: "Anthropic",
    provider_type: "anthropic" as const,
  } satisfies LlmModelWithProvider,
  openai: {
    id: "model-uuid-openai",
    provider_id: "provider-openai",
    model_id: "gpt-4o",
    display_name: "GPT-4o",
    capabilities: ["chat", "tools"],
    is_default: false,
    status: "active" as const,
    created_at: new Date().toISOString(),
    updated_at: new Date().toISOString(),
    provider_name: "OpenAI",
    provider_type: "openai" as const,
  } satisfies LlmModelWithProvider,
};

// ============================================
// Main Page Component
// ============================================

export default function DevSessionCardPage() {
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
          <h1 className="text-3xl font-bold">SessionCard Component</h1>
          <p className="text-muted-foreground mt-2">
            Session card displays session status, summary, and metadata with info button
          </p>
          <Badge variant="outline" className="mt-2">
            Development Mode
          </Badge>
        </div>

        <ScrollArea className="h-[calc(100vh-12rem)]">
          <div className="space-y-8 pr-4">
            {/* SessionCard Section */}
            <ShowcaseSection
              title="SessionCard Variants"
              description="Different states and configurations of the SessionCard component (components/session/session-card.tsx)"
            >
              <ShowcaseItem label="Running Session">
                <SessionCard
                  session={sampleSessions.running}
                  agentId="agent-123"
                  model={sampleModels.anthropic}
                />
              </ShowcaseItem>

              <ShowcaseItem label="Idle Session">
                <SessionCard
                  session={sampleSessions.idle}
                  agentId="agent-123"
                  model={sampleModels.openai}
                />
              </ShowcaseItem>

              <ShowcaseItem label="New Session (No Title)">
                <SessionCard
                  session={sampleSessions.new}
                  agentId="agent-123"
                />
              </ShowcaseItem>

              <ShowcaseItem label="Session with Long Summary (Truncated)">
                <SessionCard
                  session={sampleSessions.longSummary}
                  agentId="agent-123"
                  model={sampleModels.anthropic}
                />
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Multiple Sessions View */}
            <ShowcaseSection
              title="List View"
              description="How SessionCards appear in a list layout"
            >
              <ShowcaseItem label="Multiple Sessions (Stacked)">
                <div className="space-y-2">
                  <SessionCard
                    session={sampleSessions.running}
                    agentId="agent-123"
                    model={sampleModels.anthropic}
                  />
                  <SessionCard
                    session={sampleSessions.idle}
                    agentId="agent-123"
                    model={sampleModels.openai}
                  />
                  <SessionCard
                    session={sampleSessions.new}
                    agentId="agent-123"
                  />
                </div>
              </ShowcaseItem>
            </ShowcaseSection>

            {/* Usage Examples */}
            <ShowcaseSection
              title="Usage Patterns"
              description="Common patterns for using SessionCard in the application"
            >
              <ShowcaseItem label="With Delete Button (as used in sessions list)">
                <div className="flex items-center gap-2">
                  <div className="flex-1">
                    <SessionCard
                      session={sampleSessions.running}
                      agentId="agent-123"
                      model={sampleModels.anthropic}
                    />
                  </div>
                  <button
                    className="p-2 rounded-md hover:bg-muted text-muted-foreground hover:text-foreground"
                    type="button"
                  >
                    <svg className="h-4 w-4" fill="none" viewBox="0 0 24 24" stroke="currentColor">
                      <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
                    </svg>
                  </button>
                </div>
              </ShowcaseItem>
            </ShowcaseSection>
          </div>
        </ScrollArea>
      </div>
    </div>
  );
}
