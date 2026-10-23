"use client";

import { type ReactNode } from "react";
import { useChatMode } from "./chat-mode-context";
import { useAgentChatChanges } from "./use-agent-chat-changes";
import { ChatChangesContext } from "./chat-change-mark";

export function AgentChatBoundary({ agentId, children }: { agentId: string; children: ReactNode }) {
  const mode = useChatMode();
  return mode ? (
    <EnabledBoundary key={agentId} agentId={agentId}>
      {children}
    </EnabledBoundary>
  ) : (
    children
  );
}

function EnabledBoundary({ agentId, children }: { agentId: string; children: ReactNode }) {
  const changes = useAgentChatChanges(agentId);
  return <ChatChangesContext.Provider value={changes}>{children}</ChatChangesContext.Provider>;
}
