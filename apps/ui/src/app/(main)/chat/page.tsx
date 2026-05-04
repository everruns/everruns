"use client";

import { Loader2 } from "lucide-react";
import { ChatPanel } from "@/components/chat/chat-panel";
import { SessionProvider } from "@/app/(main)/sessions/[sessionId]/session-context";
import { useGlobalChat } from "@/hooks/use-global-chat";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { ExperimentalPageBadge } from "@/components/ui/experimental-badge";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";

function GlobalChatEnabledContent() {
  const { sessionId, isLoading, error } = useGlobalChat();

  if (error) {
    return (
      <div className="flex h-full items-center justify-center bg-background bg-brand-dots p-6">
        <div className="w-full max-w-3xl">
          <ChatErrorAlert message={error.message} />
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

export default function GlobalChatPage() {
  const globalChatEnabled = useFeatureFlag("global_chat");

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

  return <GlobalChatEnabledContent />;
}
