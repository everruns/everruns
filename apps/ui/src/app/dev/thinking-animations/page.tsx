"use client";

/**
 * Dev page: Thinking Animation
 *
 * Preview of the production ThinkingIndicator (combo wave bar)
 * in both standalone and in-chat context.
 */

import { Bot } from "lucide-react";
import { ThinkingIndicator } from "@/components/thinking-indicator";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";

// ---------------------------------------------------------------------------
// Chat context wrapper — shows the indicator inside a mock agent message
// ---------------------------------------------------------------------------
function ChatContextMock({ children }: { children: React.ReactNode }) {
  return (
    <div className="border border-border bg-background p-3">
      {/* User message */}
      <div className="flex justify-end mb-3">
        <div className="border-r-2 border-r-accent bg-secondary/50 px-3 py-2 text-xs max-w-[70%]">
          Refactor the auth module to use JWTs
        </div>
      </div>
      {/* Agent response area */}
      <div className="flex items-start gap-2">
        <div className="mt-0.5 flex h-5 w-5 flex-shrink-0 items-center justify-center border border-border bg-primary text-primary-foreground">
          <Bot className="h-3 w-3" />
        </div>
        <div className="px-3 py-2 min-w-[120px] border-l-2 border-l-primary bg-card">
          {children}
        </div>
      </div>
    </div>
  );
}

// ---------------------------------------------------------------------------
// Page
// ---------------------------------------------------------------------------
export default function ThinkingAnimationsPage() {
  return (
    <DevPageShell
      eyebrow="Thinking Animations"
      title="Wave bar thinking indicator"
      description="Production combo variant: fast 0.7s wave bars with brand navy-to-gold color ramp. Toggle dark mode to compare both themes."
      widthClassName="max-w-2xl"
    >
      <div className="space-y-8">
        {/* Standalone preview */}
        <div className="space-y-3">
          <h2 className="text-sm font-medium text-foreground">Standalone</h2>
          <div className="flex items-center justify-center h-16 border border-dashed border-border bg-background">
            <ThinkingIndicator />
          </div>
        </div>

        {/* In-chat context */}
        <div className="space-y-3">
          <h2 className="text-sm font-medium text-foreground">In chat context</h2>
          <ChatContextMock>
            <ThinkingIndicator />
          </ChatContextMock>
        </div>

        {/* With model name */}
        <div className="space-y-3">
          <h2 className="text-sm font-medium text-foreground">With model name</h2>
          <div className="flex items-center justify-center h-16 border border-dashed border-border bg-background">
            <ThinkingIndicator model="claude-opus-4-6" />
          </div>
        </div>
      </div>
    </DevPageShell>
  );
}
