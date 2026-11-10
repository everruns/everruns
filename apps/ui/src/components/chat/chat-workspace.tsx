"use client";

// One chat at a time on the left; its Threads on the right
// (knowledge/ui/chat-mode.md). Every Platform Chat conversation, Main chat and
// side chats alike, is a coordinator: the Threads panel lists the threads the
// open chat started, grouped by what they need. Opening a thread shows it in
// the panel next to the chat, or in the page pane when Chat sits beside a page.

import { useLocale as useFormattingLocale, useUiTranslations } from "@/providers/locale-provider";

import { useEffect, useRef, useState, type ReactNode } from "react";
import { usePathname, useRouter, useSearchParams } from "next/navigation";
import { ArrowLeft, Maximize2, Minimize2, Plus, X } from "lucide-react";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import { useSession } from "@/hooks/use-sessions";
import { useSessionTasks } from "@/hooks/use-session-tasks";
import { ChatThreadView } from "@/components/chat/chat-thread-view";
import { WorkHeadline } from "@/components/chat/work-headline";
import { ChatDraft } from "@/app/(main)/chats/new/new-chat-page-client";
import { ChatThreadWorkDetail } from "@/components/chat/chat-thread-work-detail";
import { ChatWorkspaceContext } from "@/components/chat/chat-workspace-context";
import { ChatErrorAlert } from "@/components/chat/chat-error-alert";
import { useChatMode } from "@/components/chat-mode/chat-mode-context";
import { Button, LinkButton } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Skeleton } from "@/components/ui/skeleton";
import { Drawer, DrawerContent, DrawerTitle } from "@/components/ui/drawer";
import {
  THREAD_GROUPS,
  assignmentGroup,
  checklistSummary,
  coordinatorThreads,
  currentAssignmentLabel,
  isAssignment,
  threadRowTitle,
  workGroup,
} from "@/lib/chat-thread-work";
import { isChatThread } from "@/lib/chat-threads";
import { formatRelativeTime } from "@/lib/formatting";
import { cn } from "@/lib/utils";
import type { SessionTask } from "@/lib/api/types";

type WorkRow = {
  id: string;
  title: string;
  preview: string;
  group: string;
  updated: string;
  /** Coordinator thread (a child conversation) rather than other background work. */
  threadId?: string;
  /** Session whose live work report replaces the preview while it runs. */
  liveSessionId?: string;
};

export function threadRows(tasks: SessionTask[]): WorkRow[] {
  return [
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
      const group = assignmentGroup(latest);
      return {
        id: threadId,
        threadId,
        title: threadRowTitle(thread),
        group,
        preview: current ? `${current} · ${status}` : status,
        updated: latest.updated_at,
        liveSessionId: group === "Working" ? threadId : undefined,
      };
    }),
    // Other background work the chat started (tools, monitors) keeps its detail view.
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
  const { displayLocale } = useFormattingLocale();
  const ui = useUiTranslations();
  const pathname = usePathname();
  const router = useRouter();
  const searchParams = useSearchParams();
  const chatMode = useChatMode();
  const { thread, error } = usePlatformChatThread({ ensure: true });
  const [open, setOpen] = useState(false);
  const [expanded, setExpanded] = useState(false);
  const [compact, setCompact] = useState(false);
  const [search, setSearch] = useState("");
  const [openThreadId, setOpenThreadId] = useState<string>();
  const threadsButton = useRef<HTMLButtonElement>(null);
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
  // Threads belong to the chat that started them; a thread shows its siblings.
  const workOwnerId = personalChat ? currentId : coordinatorId;
  const taskId = chatRoute ? searchParams.get("task") : null;
  const useDrawer = compact || pageAware;
  const panelOpen = visible && (open || !!taskId || (!useDrawer && !!openThreadId));
  useEffect(() => {
    if (personalChat && !active.data?.archived_at && currentId) onActiveChatChange?.(currentId);
  }, [personalChat, active.data?.archived_at, currentId, onActiveChatChange]);
  useEffect(() => {
    // Switching chats starts from that chat's own thread list.
    setOpenThreadId(undefined);
    setExpanded(false);
  }, [currentId, segment]);
  const work = useSessionTasks(workOwnerId);
  const task = work.data?.find((candidate) => candidate.id === taskId);
  const rows = threadRows(work.data ?? []);
  const openCount = rows.filter((row) => row.group !== "Resolved").length;
  const matchingRows = rows.filter(
    (row) =>
      row.title.toLowerCase().includes(search.toLowerCase()) ||
      row.preview.toLowerCase().includes(search.toLowerCase()),
  );

  // The panel opens by itself when this chat starts a new thread, so delegated
  // work is visible without hunting for it. Beside a page it stays closed.
  const threadCount = coordinatorThreads(work.data ?? []).length;
  const seenThreads = useRef<{ owner?: string; count: number }>({ count: 0 });
  useEffect(() => {
    if (!work.data) return;
    const previous = seenThreads.current;
    seenThreads.current = { owner: workOwnerId, count: threadCount };
    if (previous.owner === workOwnerId && threadCount > previous.count && !useDrawer && visible) {
      setOpen(true);
    }
  }, [work.data, workOwnerId, threadCount, useDrawer, visible]);

  useEffect(() => {
    // Below this width the persistent sidebar leaves too little room for two panes.
    const media = window.matchMedia("(max-width: 1279px)");
    const update = () => setCompact(media.matches);
    update();
    media.addEventListener("change", update);
    return () => media.removeEventListener("change", update);
  }, []);

  const close = () => {
    setOpen(false);
    setExpanded(false);
    setOpenThreadId(undefined);
    if (taskId) router.push(pathname);
    requestAnimationFrame(() => threadsButton.current?.focus());
  };
  const back = () => {
    setOpen(true);
    setExpanded(false);
    setOpenThreadId(undefined);
    if (taskId) router.push(pathname);
  };
  const openThread = (id: string) => {
    if (pageAware && chatMode?.openThreadBeside) {
      // Beside a page, the right side is the page pane: the thread takes its place.
      setOpen(false);
      chatMode.openThreadBeside(id);
      return;
    }
    if (useDrawer) {
      router.push(`/chats/${id}`);
      return;
    }
    setOpenThreadId(id);
    setOpen(true);
  };
  const openWork = (id: string) => {
    const owner = workOwnerId ?? currentId ?? thread?.id;
    router.push(`/chats/${owner}?task=${encodeURIComponent(id)}`);
  };

  if (error) return <ChatErrorAlert message={error.message} />;
  if (!thread || (parentId && parentId !== thread.id && parent.isLoading))
    return <Skeleton className="m-6 h-64" />;

  const threadPane = openThreadId ? (
    <ChatThreadView
      key={openThreadId}
      threadId={openThreadId}
      coordinatorId={workOwnerId}
      threadMode
      extraActions={
        <>
          <Button variant="ghost" size="sm" onClick={back}>
            <ArrowLeft className="size-3.5" /> {ui("Threads")}
          </Button>
          {!useDrawer && (
            <Button
              variant="ghost"
              size="icon"
              aria-label={expanded ? ui("Restore split view") : ui("Expand thread panel")}
              onClick={() => setExpanded(!expanded)}
            >
              {expanded ? <Minimize2 /> : <Maximize2 />}
            </Button>
          )}
          <Button variant="ghost" size="icon" aria-label={ui("Close threads")} onClick={close}>
            <X />
          </Button>
        </>
      }
    />
  ) : null;

  const panel = threadPane ?? (
    <div className="flex h-full min-h-0 w-full flex-1 flex-col bg-background">
      <div className="flex min-h-14 items-center gap-2 border-b border-border/70 px-3 py-2">
        {taskId && (
          <Button variant="ghost" size="icon" aria-label={ui("Back to threads")} onClick={back}>
            <ArrowLeft />
          </Button>
        )}
        <h2 className="mr-auto text-sm font-semibold">{ui("Threads")}</h2>
        {!useDrawer && (
          <Button
            variant="ghost"
            size="icon"
            aria-label={expanded ? ui("Restore split view") : ui("Expand thread panel")}
            onClick={() => setExpanded(!expanded)}
          >
            {expanded ? <Minimize2 /> : <Maximize2 />}
          </Button>
        )}
        <Button variant="ghost" size="icon" aria-label={ui("Close threads")} onClick={close}>
          <X />
        </Button>
      </div>
      <div className="min-h-0 flex-1 overflow-y-auto">
        {taskId ? (
          work.isLoading ? (
            <Skeleton className="m-4 h-40" />
          ) : work.error ? (
            <ChatErrorAlert message={work.error.message} />
          ) : task ? (
            <ChatThreadWorkDetail key={task.id} task={task} />
          ) : (
            <p className="p-4 text-sm text-muted-foreground">
              {ui("This work is no longer available in your Chat.")}
            </p>
          )
        ) : (
          <div className="space-y-4 p-3">
            {rows.length > 6 && (
              <Input
                aria-label={ui("Search threads")}
                placeholder={ui("Search threads…")}
                value={search}
                onChange={(e) => setSearch(e.target.value)}
              />
            )}
            {work.error && <ChatErrorAlert message={work.error.message} />}
            {work.isLoading && <Skeleton className="h-20" />}
            {!work.isLoading && matchingRows.length === 0 && (
              <p className="py-8 text-center text-sm text-muted-foreground">
                {search
                  ? ui("No matching threads.")
                  : ui("No threads yet. Ask this chat for longer work and it starts one.")}
              </p>
            )}
            {THREAD_GROUPS.map((group) => {
              const entries = matchingRows.filter((row) => row.group === group);
              if (!entries.length) return null;
              return (
                <details key={group} open={group !== "Resolved"}>
                  <summary className="cursor-pointer px-2 py-1.5 text-xs font-medium text-muted-foreground">
                    {ui(group)} · {entries.length}
                  </summary>
                  <div className="divide-y divide-border/50">
                    {entries.map((row) => (
                      <button
                        type="button"
                        key={row.id}
                        onClick={() => (row.threadId ? openThread(row.threadId) : openWork(row.id))}
                        className="flex w-full min-w-0 items-center gap-3 px-3 py-3 text-left hover:bg-muted/50 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
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
                          {formatRelativeTime(row.updated, displayLocale)}
                        </span>
                      </button>
                    ))}
                  </div>
                </details>
              );
            })}
          </div>
        )}
      </div>
    </div>
  );
  return (
    <ChatWorkspaceContext.Provider
      value={{
        sessionId: workOwnerId ?? currentId ?? thread.id,
        openTask: (target) =>
          isAssignment(target) ? openThread(target.links!.child_session_id!) : openWork(target.id),
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
                    aria-label={ui("New side chat")}
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
                    {ui("Threads")}
                    {openCount > 0 && (
                      <span className="tabular-nums text-muted-foreground">· {openCount}</span>
                    )}
                  </Button>
                </>
              }
            />
          )}
        </div>
        {useDrawer ? (
          <Drawer open={panelOpen} onOpenChange={(value) => !value && close()}>
            <DrawerContent
              className="w-full max-w-none gap-0 p-0 sm:max-w-md"
              showCloseButton={false}
              finalFocus={threadsButton}
            >
              <DrawerTitle className="sr-only">{ui("Threads")}</DrawerTitle>
              {panel}
            </DrawerContent>
          </Drawer>
        ) : (
          panelOpen && (
            <section
              aria-label={ui("Threads")}
              className={cn(
                "relative z-10 hidden min-h-0 min-w-0 overflow-hidden border-l border-border bg-background xl:flex xl:flex-col",
                expanded ? "flex-1" : openThreadId ? "w-1/2 min-w-[420px]" : "w-[420px]",
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
