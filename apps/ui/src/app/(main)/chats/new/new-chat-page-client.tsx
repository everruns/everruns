"use client";

// The new-chat surface. Its empty-state frame is the only centred message here,
// so an org with no model to chat with replaces it outright rather than showing
// a counterpart picker that cannot lead anywhere.

import { MessageCircle, Sparkles } from "lucide-react";
import { NewChatForm } from "@/components/chat/new-chat-form";
import {
  NO_INTELLIGENCE_DESCRIPTION,
  NO_INTELLIGENCE_TITLE,
  NoIntelligenceAction,
} from "@/components/chat/no-intelligence-notice";
import { BackLink, EmptyState, PageContainer } from "@/components/layout";
import { useIntelligenceStatus } from "@/hooks/use-intelligence";

export default function NewChatPageClient() {
  const intelligence = useIntelligenceStatus();
  const noIntelligence = !intelligence.isLoading && !intelligence.available;

  return (
    <PageContainer>
      <BackLink href="/chats">Chats</BackLink>
      {noIntelligence ? (
        <EmptyState
          className="mt-6"
          icon={<Sparkles />}
          title={NO_INTELLIGENCE_TITLE}
          description={NO_INTELLIGENCE_DESCRIPTION}
          action={<NoIntelligenceAction canManage={intelligence.canManage} />}
        />
      ) : (
        <EmptyState
          className="mt-6"
          icon={<MessageCircle />}
          title="New chat"
          description="A thread is bound to one agent or harness for its lifetime. Pick the counterpart you want to talk to."
          action={<NewChatForm />}
        />
      )}
    </PageContainer>
  );
}
