"use client";

import { useMemo, useRef } from "react";
import { ChatMessageList } from "@/components/chat/chat-message-list";
import { ChatNavRail, type ChatNavAnchor } from "@/components/chat/chat-nav-rail";
import { useMessageScrollerVisibility, useTurnKeyboardNavigation } from "@/hooks";
import type { Event, InputMessageData, OutputMessageCompletedData } from "@/lib/api/types";
import { getEventData, getTextFromContent } from "@/lib/api/types";

const SESSION_ID = "session-nav-rail-preview";

const TURNS: Array<{ prompt: string; reply: string }> = [
  {
    prompt: "Summarize the new shadcn chat components.",
    reply:
      "The June 2026 release adds MessageScroller, Message, Bubble, Attachment, and Marker. MessageScroller owns scroll behavior — anchored turns, streamed replies, jump-to-message, and visibility tracking — without owning your data.",
  },
  {
    prompt: "How does the navigation rail work?",
    reply:
      "It reads useMessageScrollerVisibility() for the current anchor and visible message ids, then renders a gutter of turn markers. Hover a marker to preview the turn; click to jump straight to it.",
  },
  {
    prompt: "What keeps long transcripts fast?",
    reply:
      "Rows use content-visibility: auto with contain-intrinsic-size, so the browser skips paint for off-screen turns while keeping them in the DOM for find-in-page and accessibility. Scroll state is tracked imperatively via data-* attributes.",
  },
  {
    prompt: "Can we keep our own design system?",
    reply:
      "Yes — we adopt the hook shape and the rail idea, reimplemented on Base UI and Slate. Sharp corners, gold accent for the active turn, border tones for the rest. No component-library dependency.",
  },
  {
    prompt: "Show me the active-turn highlight while I scroll.",
    reply:
      "Scroll the transcript: the marker aligned with the turn under the reading line fills with the gold accent, and it persists after that turn scrolls above the viewport.",
  },
];

function buildEvents(): Event[] {
  const events: Event[] = [];
  TURNS.forEach((turn, index) => {
    const ts = `2026-06-01T00:0${index}:00Z`;
    events.push({
      id: `input-${index}`,
      type: "input.message",
      ts,
      session_id: SESSION_ID,
      context: {},
      data: {
        message: {
          id: `msg-input-${index}`,
          session_id: SESSION_ID,
          sequence: index * 2,
          role: "user",
          content: [{ type: "text", text: turn.prompt }],
          tool_call_id: null,
          created_at: ts,
        },
      },
    });
    events.push({
      id: `output-${index}`,
      type: "output.message.completed",
      ts,
      session_id: SESSION_ID,
      context: { turn_id: `turn-${index}` },
      data: {
        message: {
          id: `msg-output-${index}`,
          session_id: SESSION_ID,
          sequence: index * 2 + 1,
          role: "assistant",
          content: [{ type: "text", text: turn.reply }],
          tool_call_id: null,
          created_at: ts,
        },
      },
    });
  });
  return events;
}

export function DevChatNavRailPreview() {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const events = useMemo(buildEvents, []);
  const emptyToolResults = useMemo(() => new Map(), []);
  const emptyToolProgress = useMemo(() => new Map(), []);
  const emptyToolOutputs = useMemo(() => new Map(), []);

  const getMessageText = (data: InputMessageData | OutputMessageCompletedData) =>
    getTextFromContent(data.message?.content ?? []);

  const navAnchors = useMemo<ChatNavAnchor[]>(() => {
    const anchors: ChatNavAnchor[] = [];
    for (const event of events) {
      if (event.type !== "input.message") continue;
      const data = getEventData(event, "input.message");
      if (!data) continue;
      const text = getMessageText(data).trim();
      anchors.push({ id: event.id, label: text.length > 80 ? `${text.slice(0, 80)}…` : text });
    }
    return anchors;
  }, [events]);

  const { currentAnchorId, scrollToAnchor } = useMessageScrollerVisibility(
    scrollContainerRef,
    navAnchors.length,
  );

  const navAnchorIds = useMemo(() => navAnchors.map((anchor) => anchor.id), [navAnchors]);
  useTurnKeyboardNavigation({
    anchorIds: navAnchorIds,
    currentAnchorId,
    onNavigate: scrollToAnchor,
  });

  return (
    <div className="overflow-hidden border border-border/70 bg-background">
      <div className="relative flex min-h-0 flex-col" style={{ height: 420 }}>
        <div
          ref={scrollContainerRef}
          className="relative flex-1 overflow-y-auto bg-background bg-brand-dots px-3 py-4 sm:px-4"
        >
          <ChatMessageList
            events={events}
            chatEvents={events}
            sessionId={SESSION_ID}
            toolResultsMap={emptyToolResults}
            toolProgressMap={emptyToolProgress}
            toolOutputMap={emptyToolOutputs}
            eventsLoading={false}
            hasMoreEvents={false}
            loadingOlderEvents={false}
            getMessageText={getMessageText}
            getToolCalls={() => []}
          />
        </div>
        <ChatNavRail
          anchors={navAnchors}
          currentAnchorId={currentAnchorId}
          onJump={scrollToAnchor}
        />
      </div>
    </div>
  );
}
