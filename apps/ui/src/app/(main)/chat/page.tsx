"use client";

import { Loader2 } from "lucide-react";
import { ChatPanel } from "@/components/chat/chat-panel";
import { SessionProvider } from "@/app/(main)/sessions/[sessionId]/session-context";
import { useGlobalChat } from "@/hooks/use-global-chat";

export default function GlobalChatPage() {
  const { sessionId, isLoading, error } = useGlobalChat();

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
    <div className="flex flex-col h-full">
      <SessionProvider sessionId={sessionId}>
        <ChatPanel />
      </SessionProvider>
    </div>
  );
}
