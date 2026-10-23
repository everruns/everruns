"use client";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import ChatThreadPage from "./[threadId]/page";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { Skeleton } from "@/components/ui/skeleton";
import { useEffect, useMemo } from "react";
import { useRouter } from "next/navigation";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";
export default function ChatPage() {
  const mode = useChatMode();
  return mode ? <ChatModeEntry /> : <StandaloneChat />;
}

function ChatModeEntry() {
  const mode = useChatMode();
  const router = useRouter();
  const setOpen = mode?.setOpen;
  useEffect(() => {
    setOpen?.(true);
    router.replace("/agents");
  }, [router, setOpen]);
  return <Skeleton className="m-6 h-64" />;
}

function StandaloneChat() {
  const { thread, error } = usePlatformChatThread({ ensure: true });
  const params = useMemo(() => Promise.resolve({ threadId: thread?.id ?? "" }), [thread?.id]);
  if (error) return <ChatErrorAlert message={error.message} />;
  if (!thread) return <Skeleton className="m-6 h-64" />;
  return <ChatThreadPage params={params} />;
}
