"use client";

import { useEffect, useRef, useState } from "react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { ArrowLeft, Maximize2, MessageSquare, Minimize2, Plus, X } from "lucide-react";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import { ChatThreadView } from "@/components/chat/chat-thread-view";
import { ChatDraft } from "@/app/(main)/chats/new/new-chat-page-client";
import { ChatThreadWorkDetail } from "@/components/chat/chat-thread-work-detail";
import { ChatWorkspaceContext } from "@/components/chat/chat-workspace-context";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { Button, LinkButton } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Drawer, DrawerContent, DrawerTitle } from "@/components/ui/drawer";
import {
  THREAD_GROUPS,
  assignmentGroup,
  checklistSummary,
  conversationGroup,
  coordinatorThreads,
  isAssignment,
  workGroup,
} from "@/lib/chat-thread-work";
import { threadTitle } from "@/lib/chat-threads";
import { formatRelativeTime } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import type { Session, SessionTask } from "@/lib/api/types";

type WorkRow = {
  id: string;
  title: string;
  preview: string;
  group: string;
  href: string;
  updated: string;
};
function rows(conversations: Session[], tasks: SessionTask[]): WorkRow[] {
  return [
    ...conversations.map((session) => ({
      id: session.id,
      title: threadTitle(session),
      preview: session.output_preview ?? session.preview ?? "",
      group: conversationGroup(session),
      href: `/chats/${session.id}`,
      updated: session.updated_at,
    })),
    // Coordinator threads: one row per thread, opened as a conversation.
    ...coordinatorThreads(tasks).map(({ threadId, latest }) => ({
      id: threadId,
      title: latest.display_name || "Thread",
      group: assignmentGroup(latest),
      preview:
        latest.input_request?.prompt ??
        latest.error?.message ??
        checklistSummary(latest) ??
        latest.summary ??
        latest.state,
      href: `/chats/${threadId}`,
      updated: latest.updated_at,
    })),
    ...tasks
      .filter((task) => !isAssignment(task))
      .map((task) => ({
        id: task.id,
        title: task.display_name || "Background work",
        group: workGroup(task),
        preview:
          task.input_request?.prompt ??
          task.error?.message ??
          task.summary ??
          task.state_detail ??
          task.state,
        href: `/chats?task=${encodeURIComponent(task.id)}`,
        updated: task.updated_at,
      })),
  ].sort((a, b) => b.updated.localeCompare(a.updated));
}

export function ChatWorkspace() {
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();
  const { thread, error } = usePlatformChatThread({ ensure: true });
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [compact, setCompact] = useState(false);
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const threadsButton = useRef<HTMLButtonElement>(null);
  const segment = pathname.replace(/^\/chats\/?/, "").split("/")[0];
  const conversationId =
    segment && !["new", "history"].includes(segment) && segment !== thread?.id ? segment : null;
  const taskId = searchParams.get("task");
  const selected = segment === "new" || !!conversationId || !!taskId;
  const panelOpen = open || selected || segment === "history";
  const conversations = useChatThreads({
    enabled: panelOpen,
    includeArchived: true,
    limit: 20,
    offset,
    search,
  });
  // One existing task subscription for the permanent conversation, never one per list row.
  const work = useSessionTasks(panelOpen ? thread?.id : undefined);
  const task = work.data?.find((candidate) => candidate.id === taskId);
  const workRows = rows(conversations.threads, work.data ?? []);
  const matchingRows = workRows.filter(
    (row) =>
      row.title.toLowerCase().includes(search.toLowerCase()) ||
      row.preview.toLowerCase().includes(search.toLowerCase()),
  );

  useEffect(() => {
    // Below this width the persistent sidebar leaves too little room for two composers.
    const media = window.matchMedia("(max-width: 1279px)");
    const update = () => setCompact(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  const close = () => {
    setOpen(false);
    setExpanded(false);
    if (pathname !== "/chats" || taskId) router.push("/chats");
  };
  const back = () => {
    setOpen(true);
    setExpanded(false);
    router.push("/chats");
  };

  if (error) return <ChatErrorAlert message={error.message} />;
  if (!thread) return <Skeleton className="m-6 h-64" />;

  const panel = (
    <div className="flex h-full min-h-0 flex-col bg-background">
      <div className="flex items-center gap-2 border-b border-border/70 px-3 py-2">
        {selected && (
          <Button variant="ghost" size="icon" aria-label="Back to threads" onClick={back}>
            <ArrowLeft />
          </Button>
        )}
        <h2 className="mr-auto text-sm font-semibold">Threads</h2>
        <LinkButton href="/chats/new" variant="ghost" size="icon" aria-label="New thread">
          <Plus />
        </LinkButton>
        {!compact && (
          <Button
            variant="ghost"
            size="icon"
            aria-label={expanded ? "Restore split view" : "Expand thread panel"}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? <Minimize2 /> : <Maximize2 />}
          </Button>
        )}
        <Button variant="ghost" size="icon" aria-label="Close threads" onClick={close}>
          <X />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {segment === "new" ? (
          <ChatDraft threadMode />
        ) : conversationId ? (
          <ChatThreadView threadId={conversationId} threadMode coordinatorId={thread.id} />
        ) : taskId ? (
          work.isLoading ? (
            <Skeleton className="m-4 h-40" />
          ) : work.error ? (
            <ChatErrorAlert message={work.error.message} />
          ) : task ? (
            <ChatThreadWorkDetail key={task.id} task={task} />
          ) : (
            <p className="p-4 text-sm text-muted-foreground">
              This work is no longer available in your Chat.
            </p>
          )
        ) : (
          <div className="space-y-4 p-3">
            <Input
              aria-label="Search threads"
              placeholder="Search threads…"
              value={search}
              onChange={(e) => {
                setSearch(e.target.value);
                setOffset(0);
              }}
            />
            {(conversations.error || work.error) && (
              <ChatErrorAlert message={(conversations.error ?? work.error)!.message} />
            )}
            {(conversations.isLoading || work.isLoading) && <Skeleton className="h-20" />}
            {!conversations.isLoading && !work.isLoading && matchingRows.length === 0 && (
              <p className="py-8 text-center text-sm text-muted-foreground">
                {search
                  ? "No matching threads."
                  : "No threads yet. Start one here, or ask Chat to do some work."}
              </p>
            )}
            {THREAD_GROUPS.map((group) => {
              const entries = matchingRows.filter((row) => row.group === group);
              if (!entries.length) return null;
              return (
                <details key={group} open={group !== "Resolved"}>
                  <summary className="cursor-pointer bg-muted/60 px-2 py-1.5 text-xs font-medium">
                    {group} <span className="text-muted-foreground">{entries.length}</span>
                  </summary>
                  <div className="divide-y divide-border/50">
                    {entries.map((row) => (
                      <Link
                        href={row.href}
                        key={row.id}
                        prefetch={false}
                        className="flex min-w-0 items-center gap-3 px-3 py-3 hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                      >
                        <div className="min-w-0 flex-1">
                          <div className="truncate text-sm font-medium">{row.title}</div>
                          <div className="truncate text-xs text-muted-foreground">
                            {row.preview || "Ready to continue"}
                          </div>
                        </div>
                        <span className="shrink-0 text-[11px] text-muted-foreground">
                          {formatRelativeTime(row.updated)}
                        </span>
                      </Link>
                    ))}
                  </div>
                </details>
              );
            })}
            {(conversations.total ?? 0) > 20 && (
              <div className="flex items-center justify-between gap-2 border-t border-border pt-3 text-xs text-muted-foreground">
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={offset === 0}
                  onClick={() => setOffset(Math.max(0, offset - 20))}
                >
                  Previous
                </Button>
                <span>
                  {offset + 1}–{Math.min(offset + 20, conversations.total!)} of{" "}
                  {conversations.total} conversations
                </span>
                <Button
                  variant="ghost"
                  size="sm"
                  disabled={offset + 20 >= conversations.total!}
                  onClick={() => setOffset(offset + 20)}
                >
                  Next
                </Button>
              </div>
            )}
          </div>
        )}
      </div>
    </div>
  );
  return (
    <ChatWorkspaceContext.Provider
      value={{
        sessionId: thread.id,
        openTask: (id) => router.push(`/chats?task=${encodeURIComponent(id)}`),
      }}
    >
      <div className="flex h-full min-h-0 min-w-0">
        <div
          className={cn("min-h-0 min-w-0 flex-1", !compact && panelOpen && expanded && "hidden")}
        >
          <ChatThreadView
            threadId={thread.id}
            extraActions={
              <Button
                ref={threadsButton}
                variant="outline"
                size="sm"
                aria-expanded={panelOpen}
                onClick={() => (panelOpen ? close() : setOpen(true))}
              >
                <MessageSquare />
                Threads
              </Button>
            }
          />
        </div>
        {compact ? (
          <Drawer open={panelOpen} onOpenChange={(value) => !value && close()}>
            <DrawerContent
              className="w-full max-w-none gap-0 p-0 sm:max-w-none"
              showCloseButton={false}
              finalFocus={threadsButton}
            >
              <DrawerTitle className="sr-only">Threads</DrawerTitle>
              {panel}
            </DrawerContent>
          </Drawer>
        ) : (
          panelOpen && (
            <section
              aria-label="Threads"
              className={cn(
                "hidden min-h-0 min-w-0 border-l border-border xl:block",
                expanded ? "flex-1" : "w-1/2 min-w-[360px]",
              )}
            >
              {panel}
            </section>
          )
        )}
      </div>
    </ChatWorkspaceContext.Provider>
  );
}
