"use client";

import { MessageCircle } from "lucide-react";
import { ChatsDisabled } from "@/components/chat/chats-disabled";
import { NewChatForm } from "@/components/chat/new-chat-form";
import { BackLink, EmptyState, PageContainer } from "@/components/layout";
import { useFeatureFlag } from "@/providers/feature-flags-provider";

export default function NewChatPageClient() {
  const chatsEnabled = useFeatureFlag("global_chat");
  if (!chatsEnabled) return <ChatsDisabled />;

  return (
    <PageContainer>
      <BackLink href="/chats">Chats</BackLink>
      <EmptyState
        className="mt-6"
        icon={<MessageCircle />}
        title="New chat"
        description="A thread is bound to one agent for its lifetime. Pick the agent you want to talk to."
        action={<NewChatForm />}
      />
    </PageContainer>
  );
}
