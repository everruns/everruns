"use client";
import { use } from "react";
import { ChatThreadView } from "@/components/chat/chat-thread-view";
export default function ChatThreadPage({ params }: { params: Promise<{ threadId: string }> }) {
  const { threadId } = use(params);
  return <ChatThreadView threadId={threadId} />;
}
