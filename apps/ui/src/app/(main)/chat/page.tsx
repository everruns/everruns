"use client";

import { Loader2 } from "lucide-react";
import { ChatPanel } from "@/components/chat/chat-panel";
import { SessionProvider } from "@/app/(main)/sessions/[sessionId]/session-context";
import { useGlobalChat } from "@/hooks/use-global-chat";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { ExperimentalPageBadge } from "@/components/ui/experimental-badge";

export default function GlobalChatPage() {
  const globalChatEnabled = useFeatureFlag("global_chat");
  const { sessionId, isLoading, error } = useGlobalChat();

  if (!globalChatEnabled) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">Global Chat is not enabled</p>
          <p className="text-sm">This feature is currently disabled.</p>
        </div>
      </div>
    );
  }

  if (isLoading || !sessionId) {
    return (
      <div className="flex h-full items-center justify-center">
        <Loader2 className="h-8 w-8 animate-spin text-muted-foreground" />
      </div>
    );
  }

  if (error) {
    return (
      <div className="flex h-full items-center justify-center">
        <div className="text-center text-muted-foreground">
          <p className="text-lg font-medium">Failed to load chat</p>
          <p className="text-sm">{error.message}</p>
        </div>
      </div>
    );
  }

  return (
    <div className="flex h-full flex-col bg-background bg-brand-dots">
      <div className="flex items-center justify-between border-b border-border/70 bg-background/70 px-6 py-3.5 backdrop-blur-[1px]">
        <h1 className="flex items-center gap-3 text-xl font-semibold tracking-tight">
          Chat
          <ExperimentalPageBadge />
        </h1>
      </div>
      <SessionProvider sessionId={sessionId}>
        <ChatPanel />
      </SessionProvider>
    </div>
  );
}
