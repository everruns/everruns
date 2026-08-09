"use client";

// The read-only half of a conversation: history, streaming output, and turn
// navigation, with no composer and no mutating call of any kind.
//
// Extracted from `ChatPanel` (EVE-854) so the session detail page — a recording,
// not a workspace — can render the same transcript the chat surface renders.
// `ChatPanel` keeps this component and adds the composer on top, so the two
// surfaces cannot drift.

import { useMemo, type ReactNode } from "react";
import { ArrowDown, Bot } from "lucide-react";
import { getEventData } from "@/lib/api/types";
import { cn } from "@/lib/utils";
import { useSessionContext } from "@/app/(main)/sessions/[sessionId]/session-context";
import {
  useMessageScrollerVisibility,
  useScrollManager,
  useSessionParticipants,
  useTurnKeyboardNavigation,
} from "@/hooks";
import { chatSurfaceStyles } from "@/components/chat/chat-surface";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { ChatNavRail, type ChatNavAnchor } from "@/components/chat/chat-nav-rail";
import { StreamingMessage } from "@/components/streaming-message";
import { ThinkingIndicator } from "@/components/thinking-indicator";
import { useLocale } from "@/providers/locale-provider";

export function SessionTranscript({
  /** Rendered inside the scroll container, below the transcript (e.g. composer errors). */
  footer,
}: {
  footer?: ReactNode;
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
    streamingMessageId,
    streamingIteration,
    hasMoreEvents,
    loadingOlderEvents,
    loadOlderEvents,
    getMessageText,
    getToolCalls,
  } = useSessionContext();

  const { data: participants } = useSessionParticipants(sessionId);

  const { scrollContainerRef, messagesEndRef, hasNewMessages, dismissNewMessages, handleScrollUp } =
    useScrollManager({
      eventCount: chatEvents.length,
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
          "relative flex-1 overflow-y-auto bg-background bg-brand-dots px-3 py-4 sm:px-4",
          !eventsLoading && chatEvents.length === 0 && "flex flex-col justify-end",
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
        />

        {(isThinking || streamingText) && (
          <div className="mt-4 flex justify-start">
            <div className={chatSurfaceStyles.agentMessageRow}>
              <div className={chatSurfaceStyles.agentIcon}>
                <Bot className="h-3 w-3" />
              </div>
              <div className={chatSurfaceStyles.agentMessage}>
                {streamingIteration && streamingIteration > 1 && (
                  <div className="mb-1 text-xs text-muted-foreground">
                    {t("iteration", { value: streamingIteration })}
                  </div>
                )}
                {isThinking && !streamingText ? (
                  <ThinkingIndicator />
                ) : streamingText && streamingMessageId ? (
                  <StreamingMessage messageId={streamingMessageId} text={streamingText} />
                ) : null}
              </div>
            </div>
          </div>
        )}

        {footer}

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
