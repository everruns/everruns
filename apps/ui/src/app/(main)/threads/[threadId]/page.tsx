"use client";

// A thread a chat started, shown as a page: in the chat-first layout it sits
// beside the chat that owns it. It is a route (not shell state) so the browser
// back button returns to the page it was opened from (knowledge/ui/chat-mode.md).
import { use } from "react";
import { useRouter } from "next/navigation";
import { ArrowLeft, Maximize2 } from "lucide-react";
import { useUiTranslations } from "@/providers/locale-provider";
import { ChatThreadView } from "@/components/chat/chat-thread-view";
import { Button, LinkButton } from "@/components/ui/button";
import { Skeleton } from "@/components/ui/skeleton";
import { useSession } from "@/hooks/use-sessions";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";

export default function ThreadPage({ params }: { params: Promise<{ threadId: string }> }) {
  const { threadId } = use(params);
  const ui = useUiTranslations();
  const router = useRouter();
  const { data: session, isLoading } = useSession(threadId);
  const { thread: mainChat } = usePlatformChatThread();
  const { threads, isLoading: chatsLoading } = useChatThreads({ includeArchived: true });
  const parent = session?.parent_session_id ?? undefined;
  // Only one of the person's own chats counts as the coordinator.
  const coordinatorId =
    parent && (parent === mainChat?.id || threads.some((chat) => chat.id === parent))
      ? parent
      : undefined;

  if (isLoading || (parent && chatsLoading && !coordinatorId)) {
    return (
      <div className="p-6">
        <Skeleton className="mb-4 h-8 w-1/3" />
        <Skeleton className="h-64 w-full" />
      </div>
    );
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      <ChatThreadView
        threadId={threadId}
        coordinatorId={coordinatorId}
        threadMode
        extraActions={
          <>
            <Button
              variant="ghost"
              size="sm"
              onClick={() =>
                // Opened from a link with no app history: go to Overview instead of leaving.
                window.history.length > 1 ? router.back() : router.push("/home")
              }
            >
              <ArrowLeft className="size-3.5" /> {ui("Back")}
            </Button>
            <LinkButton
              variant="ghost"
              size="icon-sm"
              href={`/chats/${threadId}`}
              aria-label={ui("Open thread full screen")}
            >
              <Maximize2 className="size-3.5" />
            </LinkButton>
          </>
        }
      />
    </div>
  );
}
