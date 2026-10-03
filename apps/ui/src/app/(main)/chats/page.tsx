"use client";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import ChatThreadPage from "./[threadId]/page";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { Skeleton } from "@/components/ui/skeleton";
import { useMemo } from "react";
export default function ChatPage() {
  const { thread, error } = usePlatformChatThread({ ensure: true });
  const params = useMemo(() => Promise.resolve({ threadId: thread?.id ?? "" }), [thread?.id]);
  if (error) return <ChatErrorAlert message={error.message} />;
  if (!thread) return <Skeleton className="m-6 h-64" />;
  return <ChatThreadPage params={params} />;
}
