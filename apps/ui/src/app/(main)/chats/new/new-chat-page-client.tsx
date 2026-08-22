"use client";

import { MessageCircle } from "lucide-react";
import { NewChatForm } from "@/components/chat/new-chat-form";
import { BackLink, EmptyState, PageContainer } from "@/components/layout";

export default function NewChatPageClient() {
  return (
    <PageContainer>
      <BackLink href="/chats">Chats</BackLink>
      <EmptyState
        className="mt-6"
        icon={<MessageCircle />}
        title="New chat"
        description="A thread is bound to one agent or harness for its lifetime. Pick the counterpart you want to talk to."
        action={<NewChatForm />}
      />
    </PageContainer>
  );
}
