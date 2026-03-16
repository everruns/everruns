"use client";

import Link from "next/link";
import {
  FlaskConical,
  MessageSquare,
  LayoutList,
  ArrowRight,
  Star,
  FileText,
  Gauge,
  FileCode,
} from "lucide-react";

// Check if we're in development mode
const isDev = process.env.NODE_ENV === "development";

const devPages = [
  {
    title: "Chat Components",
    description: "Chat-specific: messages, tool calls, todo lists, image attachments",
    href: "/dev/chat-components",
    icon: MessageSquare,
  },
  {
    title: "Tool Activity",
    description: "Grouped tool execution with animations, summaries, and todo progress cards",
    href: "/dev/tool-activity",
    icon: MessageSquare,
  },
  {
    title: "Session Components",
    description: "Session-level: token usage, status badges, session headers",
    href: "/dev/session-components",
    icon: Gauge,
  },
  {
    title: "Streamdown Message",
    description:
      "Streaming-optimized markdown renderer with GFM, code highlighting, and incomplete syntax handling",
    href: "/dev/markdown",
    icon: FileText,
  },
  {
    title: "SessionCard Component",
    description: "Session card with status indicators, summary display, and metadata tooltips",
    href: "/dev/session-card",
    icon: LayoutList,
  },
  {
    title: "Model Picker",
    description: "LLM model selection component with favorite models support",
    href: "/dev/model-picker",
    icon: Star,
  },
  {
    title: "File Previews",
    description: "Code, CSV, JSON, markdown, and image preview components with Shiki highlighting",
    href: "/dev/file-previews",
    icon: FileCode,
  },
];

export default function DevPage() {
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
    <div className="min-h-screen bg-background bg-brand-dots px-4 py-8">
      <div className="mx-auto max-w-6xl">
        <div className="mb-8 max-w-2xl space-y-2">
          <div className="flex items-center gap-3">
            <FlaskConical className="h-7 w-7 text-primary" />
            <p className="text-[11px] uppercase tracking-[0.3em] text-muted-foreground">
              Developer Tools
            </p>
          </div>
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">
            UI reference pages
          </h1>
          <p className="text-sm leading-6 text-muted-foreground">
            Development-only previews for the application’s canonical component styles and
            behaviors.
          </p>
        </div>

        <div className="grid gap-4 md:grid-cols-2 lg:grid-cols-3">
          {devPages.map((page) => (
            <Link key={page.href} href={page.href}>
              <div className="flex h-full cursor-pointer flex-col justify-between border border-border bg-card px-5 py-5 transition-colors hover:border-primary">
                <div className="flex items-center justify-between">
                  <page.icon className="h-5 w-5 text-muted-foreground" />
                  <ArrowRight className="h-4 w-4 text-muted-foreground" />
                </div>
                <div className="mt-8">
                  <h2 className="text-lg font-medium text-foreground">{page.title}</h2>
                  <p className="mt-2 text-sm leading-6 text-muted-foreground">{page.description}</p>
                </div>
              </div>
            </Link>
          ))}
        </div>
      </div>
    </div>
  );
}
