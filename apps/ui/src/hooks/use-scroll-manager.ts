// Scroll management hook for chat panels
//
// Decision: Consolidates 6 scroll-related refs and all scroll logic into a single hook.
// Handles: auto-scroll on new content, "new messages" indicator, scroll position
// preservation when older events are prepended, and infinite scroll-up detection.

import { useRef, useState, useCallback, useEffect, useLayoutEffect } from "react";

const NEAR_BOTTOM_THRESHOLD = 100;
const SCROLL_UP_TRIGGER_PX = 200;

interface UseScrollManagerOptions {
  /** Total event count — used to detect new events vs prepended older ones */
  eventCount: number;
  /** Whether initial event loading is complete */
  eventsLoaded: boolean;
  /** Whether there are more older events to load */
  hasMoreEvents: boolean;
  /** Whether older events are currently loading */
  loadingOlderEvents: boolean;
  /** Load older events callback */
  loadOlderEvents: () => void;
  /** Session ID — resets state on change */
  sessionId: string;
  /** Dependencies that trigger auto-scroll when near bottom (e.g., streaming text) */
  scrollDeps?: unknown[];
}

export function useScrollManager({
  eventCount,
  eventsLoaded,
  hasMoreEvents,
  loadingOlderEvents,
  loadOlderEvents,
  sessionId,
  scrollDeps = [],
}: UseScrollManagerOptions) {
  const scrollContainerRef = useRef<HTMLDivElement>(null);
  const messagesEndRef = useRef<HTMLDivElement>(null);
  const isNearBottomRef = useRef(true);
  const prevEventCountRef = useRef(0);
  const initialScrollDoneRef = useRef(false);
  const prevScrollHeightRef = useRef<number | null>(null);
  const [hasNewMessages, setHasNewMessages] = useState(false);

  const checkIsNearBottom = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container) return true;
    const { scrollTop, scrollHeight, clientHeight } = container;
    return scrollHeight - scrollTop - clientHeight <= NEAR_BOTTOM_THRESHOLD;
  }, []);

  const scrollToBottom = useCallback((behavior: ScrollBehavior = "smooth") => {
    messagesEndRef.current?.scrollIntoView({ behavior });
  }, []);

  // Track near-bottom state on scroll
  useEffect(() => {
    const container = scrollContainerRef.current;
    if (!container) return;

    const handleScroll = () => {
      const nearBottom = checkIsNearBottom();
      isNearBottomRef.current = nearBottom;
      if (nearBottom) {
        setHasNewMessages(false);
      }
    };

    container.addEventListener("scroll", handleScroll, { passive: true });
    return () => container.removeEventListener("scroll", handleScroll);
  }, [checkIsNearBottom]);

  // Initial scroll to bottom once events load
  useEffect(() => {
    if (eventsLoaded && eventCount > 0 && !initialScrollDoneRef.current) {
      initialScrollDoneRef.current = true;
      requestAnimationFrame(() => {
        scrollToBottom("instant");
      });
    }
  }, [eventsLoaded, eventCount, scrollToBottom]);

  // Reset on session change
  useEffect(() => {
    initialScrollDoneRef.current = false;
    isNearBottomRef.current = true;
    setHasNewMessages(false);
    prevEventCountRef.current = 0;
  }, [sessionId]);

  // Auto-scroll on new events or streaming (only when near bottom)
  useEffect(() => {
    if (!initialScrollDoneRef.current) return;

    // Always track event count so prepend-only updates don't cause
    // a stale hasNewEvents on the next scrollDeps change.
    const hasNewEvents = eventCount > prevEventCountRef.current;
    prevEventCountRef.current = eventCount;

    if (prevScrollHeightRef.current !== null) return;

    if (isNearBottomRef.current) {
      scrollToBottom("smooth");
    } else if (hasNewEvents) {
      setHasNewMessages(true);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [eventCount, scrollToBottom, ...scrollDeps]);

  // Preserve scroll position when older events are prepended
  useLayoutEffect(() => {
    const container = scrollContainerRef.current;
    const prevHeight = prevScrollHeightRef.current;
    if (container && prevHeight !== null) {
      const newHeight = container.scrollHeight;
      container.scrollTop += newHeight - prevHeight;
      prevScrollHeightRef.current = null;
    }
  }, [eventCount]);

  // Scroll-up detection for loading older events
  const handleScrollUp = useCallback(() => {
    const container = scrollContainerRef.current;
    if (!container || !hasMoreEvents || loadingOlderEvents) return;

    if (container.scrollTop < SCROLL_UP_TRIGGER_PX) {
      prevScrollHeightRef.current = container.scrollHeight;
      loadOlderEvents();
    }
  }, [hasMoreEvents, loadingOlderEvents, loadOlderEvents]);

  const dismissNewMessages = useCallback(() => {
    scrollToBottom("smooth");
    setHasNewMessages(false);
  }, [scrollToBottom]);

  return {
    scrollContainerRef,
    messagesEndRef,
    hasNewMessages,
    dismissNewMessages,
    handleScrollUp,
    scrollToBottom,
  };
}
