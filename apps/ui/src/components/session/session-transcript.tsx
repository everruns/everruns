"use client";

// The read-only half of a conversation: history, streaming output, and turn
// navigation, with no composer and no mutating call of any kind.
//
// Extracted from `ChatPanel` (EVE-854) so the session detail page — a recording,
// not a workspace — can render the same transcript the chat surface renders.
// `ChatPanel` keeps this component and adds the composer on top, so the two
// surfaces cannot drift.

import { useMemo, type ReactNode } from "react";
import { ArrowDown } from "lucide-react";
import { getEventData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import {
  useMessageScrollerVisibility,
  useScrollManager,
  useSessionParticipants,
  useTurnKeyboardNavigation,
} from "@/hooks";
import { useThreadRuns } from "@/hooks/use-thread-runs";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { assignRunsToEvents } from "@/components/chat/run-cards";
import { ChatNavRail, type ChatNavAnchor } from "@/components/chat/chat-nav-rail";
import { StreamingMessage } from "@/components/streaming-message";
import { useLocale } from "@/providers/locale-provider";

export function SessionTranscript({
  /** Rendered inside the scroll container, below the transcript (e.g. composer errors). */
  footer,
  /** Show inline run cards for work the turns started (Chats thread surface). */
  showRunCards = false,
  /** Human chat opts into folding; session recordings show the full work log. */
  collapseWorkLog = false,
  /** Replaces the transcript's default empty state (see `ChatMessageList`). */
  emptyState,
}: {
  footer?: ReactNode;
  showRunCards?: boolean;
  collapseWorkLog?: boolean;
  emptyState?: ReactNode;
}) {
  const { t } = useLocale();
  const {
    events,
    sessionId,
    chatEvents,
    toolResultsMap,
    toolProgressMap,
    toolOutputMap,
    eventsLoading,
    isThinking,
    streamingText,
    streamingTurnId,
    streamingMessageId,
    streamingIteration,
    streamingPhase,
    hasMoreEvents,
    loadingOlderEvents,
    loadOlderEvents,
    getMessageText,
    getToolCalls,
    chatSends,
    isActive,
  } = useSessionContext();
  const transcriptEmpty = chatEvents.length === 0 && chatSends.pending.length === 0;

  const { data: participants } = useSessionParticipants(sessionId);

  // Inert unless the surface asked for run cards: no task stream, no requests.
  // See `run-cards.ts` for how a run is attributed back to the turn that started it.
  const runs = useThreadRuns(showRunCards ? sessionId : undefined);
  const runsByEventId = useMemo(
    () => (showRunCards ? assignRunsToEvents(events ?? [], runs) : undefined),
    [showRunCards, events, runs],
  );

  const { scrollContainerRef, messagesEndRef, hasNewMessages, dismissNewMessages, handleScrollUp } =
    useScrollManager({
      eventCount: chatEvents.length + chatSends.pending.length,
      eventsLoaded: !eventsLoading,
      hasMoreEvents,
      loadingOlderEvents,
      loadOlderEvents,
      sessionId,
      scrollDeps: [streamingText, isThinking],
    });

  // Turn navigation rail: one marker per user turn. Anchors must line up with
  // the `data-message-anchor` markers that ChatMessageList sets on user rows.
  const navAnchors = useMemo<ChatNavAnchor[]>(() => {
    const anchors: ChatNavAnchor[] = [];
    for (const event of chatEvents) {
      if (event.type !== "input.message") continue;
      const data = getEventData(event, "input.message");
      if (!data) continue;
      const text = getMessageText(data).trim();
      anchors.push({
        id: event.id,
        label: text.length > 80 ? `${text.slice(0, 80)}…` : text,
      });
    }
    return anchors;
  }, [chatEvents, getMessageText]);

  const { currentAnchorId, scrollToAnchor } = useMessageScrollerVisibility(
    scrollContainerRef,
    navAnchors.length,
  );

  // Keyboard turn stepping (Alt+↑/↓, or j/k outside inputs), built on the same
  // anchors and scrollToAnchor the rail uses.
  const navAnchorIds = useMemo(() => navAnchors.map((anchor) => anchor.id), [navAnchors]);
  useTurnKeyboardNavigation({
    anchorIds: navAnchorIds,
    currentAnchorId,
    onNavigate: scrollToAnchor,
  });

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollContainerRef}
        onScroll={handleScrollUp}
        className={cn(
          "relative flex-1 overflow-y-auto bg-background bg-brand-dots px-3 py-4 sm:px-6",
          !eventsLoading && transcriptEmpty && "flex flex-col justify-center",
        )}
      >
        <div
          className={cn(
            chatSurfaceStyles.column,
            !eventsLoading && transcriptEmpty && "flex flex-1 flex-col justify-center",
          )}
        >
          <ChatMessageList
            events={events}
            chatEvents={chatEvents}
            sessionId={sessionId}
            toolResultsMap={toolResultsMap}
            toolProgressMap={toolProgressMap}
            toolOutputMap={toolOutputMap}
            eventsLoading={eventsLoading}
            hasMoreEvents={hasMoreEvents}
            loadingOlderEvents={loadingOlderEvents}
            getMessageText={getMessageText}
            getToolCalls={getToolCalls}
            participants={participants}
            runsByEventId={runsByEventId}
            emptyState={emptyState}
            collapseWorkLog={collapseWorkLog}
            pendingSends={chatSends.pending}
            onRetrySend={chatSends.retry}
            sessionActive={isActive}
            streamingWork={
              (isThinking && !streamingText) || streamingPhase === "commentary"
                ? {
                    turnId: streamingTurnId ?? null,
                    text: streamingPhase === "commentary" ? streamingText : null,
                    isThinking: Boolean(isThinking && !streamingText),
                  }
                : null
            }
          />

          {streamingText && streamingPhase !== "commentary" && (
            <div className="mt-4 flex justify-start">
              <div className={chatSurfaceStyles.agentMessageRow}>
                <div className={chatSurfaceStyles.agentMessage}>
                  {streamingIteration && streamingIteration > 1 && (
                    <div className="mb-1 text-xs text-muted-foreground">
                      {t("iteration", { value: streamingIteration })}
                    </div>
                  )}
                  {streamingText && streamingMessageId ? (
                    <StreamingMessage messageId={streamingMessageId} text={streamingText} />
                  ) : null}
                </div>
              </div>
            </div>
          )}

          {footer}
        </div>

        <div ref={messagesEndRef} />

        {hasNewMessages && (
          <button
            type="button"
            onClick={dismissNewMessages}
            className={chatSurfaceStyles.floatingNotice}
          >
            <ArrowDown className="h-3 w-3" />
            {t("new_messages")}
          </button>
        )}
      </div>

      <ChatNavRail anchors={navAnchors} currentAnchorId={currentAnchorId} onJump={scrollToAnchor} />
    </div>
  );
}
