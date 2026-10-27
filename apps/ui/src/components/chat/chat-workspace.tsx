"use client";

import { useEffect, useId, useRef, useState, type ReactNode } from "react";
import Link from "next/link";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { ArrowLeft, Maximize2, MessageSquare, Minimize2, Plus, X } from "lucide-react";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import { useChatThreads } from "@/hooks/use-chat-threads";
import { useFeatureFlag } from "@/providers/feature-flags-provider";
import { useSession } from "@/hooks/use-sessions";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import { ChatThreadView } from "@/components/chat/chat-thread-view";
import { WorkHeadline } from "@/components/chat/work-headline";
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
  currentAssignmentLabel,
  isAssignment,
  threadRowTitle,
  workGroup,
} from "@/lib/chat-thread-work";
import { isChatThread, threadTitle } from "@/lib/chat-threads";
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
  /** Session whose live work report replaces the preview while it runs. */
  liveSessionId?: string;
};
function rows(conversations: Session[], tasks: SessionTask[], ownerId?: string): WorkRow[] {
  return [
    ...conversations.map((session) => ({
      id: session.id,
      title: threadTitle(session),
      preview: session.output_preview ?? session.preview ?? "",
      group: conversationGroup(session),
      href: `/chats/${session.id}`,
      updated: session.updated_at,
      liveSessionId:
        session.status === "active" || session.status === "waiting_for_tool_results"
          ? session.id
          : undefined,
    })),
    // Coordinator threads: one row per thread, opened as a conversation.
    ...coordinatorThreads(tasks).map((thread) => {
      const { threadId, latest } = thread;
      const status =
        latest.input_request?.prompt ??
        latest.error?.message ??
        checklistSummary(latest) ??
        latest.summary ??
        latest.state;
      const current = currentAssignmentLabel(thread);
      return {
        id: threadId,
        title: threadRowTitle(thread),
        group: assignmentGroup(latest),
        preview: current ? `${current} · ${status}` : status,
        href: `/chats/${threadId}`,
        updated: latest.updated_at,
      };
    }),
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
        href: `${ownerId ? `/chats/${ownerId}` : "/chats"}?task=${encodeURIComponent(task.id)}`,
        updated: task.updated_at,
      })),
  ].sort((a, b) => b.updated.localeCompare(a.updated));
}

export function ChatWorkspace({
  activeChatId,
  pageAware = false,
  visible = true,
  onPageMessage,
  onActiveChatChange,
  extraActions,
  notice,
}: {
  activeChatId?: string;
  pageAware?: boolean;
  visible?: boolean;
  onPageMessage?: () => void;
  onActiveChatChange?: (id: string) => void;
  extraActions?: ReactNode;
  notice?: ReactNode;
} = {}) {
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();
  const { thread, error } = usePlatformChatThread({ ensure: true });
  const [open, setOpen] = useState(false);
  const backgroundWork = useFeatureFlag("chat_threads");
  const [tab, setTab] = useState<"subchats" | "work">("subchats");
  const [expanded, setExpanded] = useState(false);
  const [compact, setCompact] = useState(false);
  const [search, setSearch] = useState("");
  const [offset, setOffset] = useState(0);
  const threadsButton = useRef<HTMLButtonElement>(null);
  const panelId = useId();
  const chatRoute = pathname === "/chats" || pathname.startsWith("/chats/");
  const segment = chatRoute ? pathname.slice("/chats".length).split("/")[1] : undefined;
  const conversationId = segment && !["new", "history"].includes(segment) ? segment : undefined;
  const currentId = chatRoute ? (conversationId ?? thread?.id) : (activeChatId ?? thread?.id);
  const active = useSession(currentId);
  const parentId = active.data?.parent_session_id ?? undefined;
  const parent = useSession(parentId && parentId !== thread?.id ? parentId : undefined);
  const personalChat =
    active.data &&
    isChatThread(active.data) &&
    active.data.agent_id === thread?.agent_id &&
    !parentId;
  const coordinatorId =
    parentId === thread?.id
      ? parentId
      : parent.data &&
          isChatThread(parent.data) &&
          parent.data.agent_id === thread?.agent_id &&
          !parent.data.parent_session_id
        ? parent.data.id
        : undefined;
  const workOwnerId = personalChat ? currentId : coordinatorId;
  const taskId = chatRoute ? searchParams.get("task") : null;
  const workTab = (backgroundWork && tab === "work") || !!taskId;
  const selected = !!taskId;
  const panelOpen = visible && (open || selected || segment === "history");
  const useDrawer = compact || pageAware;
  useEffect(() => {
    if (personalChat && !active.data?.archived_at && currentId) onActiveChatChange?.(currentId);
  }, [personalChat, active.data?.archived_at, currentId, onActiveChatChange]);
  useEffect(() => {
    // Selecting a conversation replaces the main surface; it never opens a second composer.
    setOpen(false);
    setExpanded(false);
  }, [currentId, segment]);
  const conversations = useChatThreads({
    enabled: panelOpen && !workTab,
    includeArchived: true,
    limit: 20,
    offset,
    search,
  });
  // Background work belongs to this Chat instance, not to a global conversation.
  const work = useSessionTasks(panelOpen && workTab ? workOwnerId : undefined);
  const task = work.data?.find((candidate) => candidate.id === taskId);
  const workRows = rows(
    workTab ? [] : conversations.threads,
    workTab ? (work.data ?? []) : [],
    workOwnerId,
  );
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
    if (taskId) router.push(pathname);
    requestAnimationFrame(() => threadsButton.current?.focus());
  };
  const back = () => {
    setOpen(true);
    setExpanded(false);
    if (taskId) router.push(pathname);
  };

  if (error) return <ChatErrorAlert message={error.message} />;
  if (!thread || (parentId && parentId !== thread.id && parent.isLoading))
    return <Skeleton className="m-6 h-64" />;

  const panel = (
    <div className="flex h-full min-h-0 w-full flex-1 flex-col bg-background">
      <div className="flex items-center gap-2 border-b border-border/70 px-3 py-2">
        {selected && (
          <Button variant="ghost" size="icon" aria-label="Back to threads" onClick={back}>
            <ArrowLeft />
          </Button>
        )}
        <div
          role="tablist"
          aria-label="Chat panels"
          className="mr-auto flex gap-1"
          onKeyDown={(event) => {
            if (!["ArrowLeft", "ArrowRight", "Home", "End"].includes(event.key)) return;
            event.preventDefault();
            const tabs = Array.from(
              event.currentTarget.querySelectorAll<HTMLButtonElement>('[role="tab"]'),
            );
            const current = tabs.indexOf(document.activeElement as HTMLButtonElement);
            const index =
              event.key === "Home"
                ? 0
                : event.key === "End"
                  ? tabs.length - 1
                  : (current + (event.key === "ArrowRight" ? 1 : -1) + tabs.length) % tabs.length;
            tabs[index]?.focus();
            tabs[index]?.click();
          }}
        >
          <Button
            role="tab"
            id={`${panelId}-subchats`}
            aria-controls={panelId}
            tabIndex={workTab ? -1 : 0}
            aria-selected={!workTab}
            variant={!workTab ? "secondary" : "ghost"}
            size="sm"
            onClick={() => {
              setTab("subchats");
              if (taskId) router.push(pathname);
            }}
          >
            Subchats
          </Button>
          {(backgroundWork || taskId) && (
            <Button
              role="tab"
              id={`${panelId}-work`}
              aria-controls={panelId}
              tabIndex={workTab ? 0 : -1}
              aria-selected={workTab}
              variant={workTab ? "secondary" : "ghost"}
              size="sm"
              onClick={() => setTab("work")}
            >
              Work
            </Button>
          )}
        </div>
        <LinkButton href="/chats/new" variant="ghost" size="icon" aria-label="New subchat">
          <Plus />
        </LinkButton>
        {!useDrawer && (
          <Button
            variant="ghost"
            size="icon"
            aria-label={expanded ? "Restore split view" : "Expand thread panel"}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? <Minimize2 /> : <Maximize2 />}
          </Button>
        )}
        <Button variant="ghost" size="icon" aria-label="Close subchats" onClick={close}>
          <X />
        </Button>
      </div>
      <div
        role="tabpanel"
        id={panelId}
        aria-labelledby={`${panelId}-${workTab ? "work" : "subchats"}`}
        className="min-h-0 flex-1 overflow-y-auto"
      >
        {taskId ? (
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
            {((!workTab && conversations.error) || (workTab && work.error)) && (
              <ChatErrorAlert message={(conversations.error ?? work.error)!.message} />
            )}
            {((!workTab && conversations.isLoading) || (workTab && work.isLoading)) && (
              <Skeleton className="h-20" />
            )}
            {!(workTab ? work.isLoading : conversations.isLoading) && matchingRows.length === 0 && (
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
                            <WorkHeadline
                              sessionId={row.liveSessionId}
                              live={!!row.liveSessionId}
                              fallback={row.preview || "Ready to continue"}
                            />
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
            {!workTab && (conversations.total ?? 0) > 20 && (
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
        sessionId: workOwnerId ?? currentId ?? thread.id,
        openTask: (id) =>
          router.push(
            `/chats/${workOwnerId ?? currentId ?? thread.id}?task=${encodeURIComponent(id)}`,
          ),
      }}
    >
      <div className="flex h-full min-h-0 min-w-0 overflow-hidden">
        {/* Isolate and clip the transcript so positioned rows cannot paint over the panel. */}
        <div
          className={cn(
            "isolate flex min-h-0 min-w-0 flex-1 flex-col overflow-hidden",
            !useDrawer && panelOpen && expanded && "hidden",
          )}
        >
          {segment === "new" ? (
            <ChatDraft />
          ) : (
            <ChatThreadView
              threadId={currentId ?? thread.id}
              coordinatorId={coordinatorId}
              threadMode={!!coordinatorId}
              pageAware={pageAware && !!personalChat && !active.data?.archived_at}
              onPageMessage={onPageMessage}
              notice={notice}
              extraActions={
                <>
                  {extraActions}
                  <LinkButton
                    href="/chats/new"
                    variant="ghost"
                    size="icon"
                    aria-label="New subchat"
                  >
                    <Plus />
                  </LinkButton>
                  <Button
                    ref={threadsButton}
                    variant="outline"
                    size="sm"
                    aria-expanded={panelOpen}
                    onClick={() => (panelOpen ? close() : setOpen(true))}
                  >
                    <MessageSquare />
                    Subchats
                  </Button>
                </>
              }
            />
          )}
        </div>
        {useDrawer ? (
          <Drawer open={panelOpen} onOpenChange={(value) => !value && close()}>
            <DrawerContent
              className="w-full max-w-none gap-0 p-0 sm:max-w-none"
              showCloseButton={false}
              finalFocus={threadsButton}
            >
              <DrawerTitle className="sr-only">Subchats</DrawerTitle>
              {panel}
            </DrawerContent>
          </Drawer>
        ) : (
          panelOpen && (
            <section
              aria-label="Subchats"
              className={cn(
                "relative z-10 hidden min-h-0 min-w-0 overflow-hidden border-l border-border bg-background xl:flex xl:flex-col",
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
