// Message-scroller visibility tracking for the chat transcript.
//
// Adopts the shape shadcn/ui's `useMessageScrollerVisibility` hook exposes
// (`currentAnchorId` + `visibleAnchorIds`), reimplemented on our own scroll
// container instead of pulling in the component library — we stay on Base UI
// and the Slate design system.
//
// Decisions:
// - Anchors are marked in the DOM with `data-message-anchor="<id>"`; the hook
//   re-queries them on demand so it needs no observer lifecycle when the
//   transcript mutates (messages stream in, older pages prepend).
// - Position is read imperatively via getBoundingClientRect on a
//   rAF-throttled scroll handler. Turn counts are small (tens, low hundreds),
//   so this stays cheap and avoids per-row React rerenders.
// - `currentAnchorId` is the last anchor at or above a reading line near the
//   top of the viewport, so it persists after an anchor scrolls off the top —
//   matching the "which turn am I reading" semantics of a navigation rail.

import { useCallback, useEffect, useRef, useState, type RefObject } from "react";

// Distance below the viewport top that marks the "reading line". The current
// anchor is the last turn whose top has crossed above this line.
const READING_LINE_OFFSET = 24;

export interface MessageScrollerVisibility {
  /** Turn currently being read (last anchor above the reading line). */
  currentAnchorId: string | null;
  /** Anchors intersecting the viewport, in document order. */
  visibleAnchorIds: string[];
  /** Scroll a given anchor to the top of the viewport. */
  scrollToAnchor: (id: string, behavior?: ScrollBehavior) => void;
}

function sameOrder(a: string[], b: string[]): boolean {
  return a.length === b.length && a.every((value, index) => value === b[index]);
}

export function useMessageScrollerVisibility(
  containerRef: RefObject<HTMLElement | null>,
  // Changes whenever the set of anchors changes, so the hook recomputes.
  anchorCount: number,
): MessageScrollerVisibility {
  const [currentAnchorId, setCurrentAnchorId] = useState<string | null>(null);
  const [visibleAnchorIds, setVisibleAnchorIds] = useState<string[]>([]);
  const frameRef = useRef<number | null>(null);

  const compute = useCallback(() => {
    const container = containerRef.current;
    if (!container) return;

    const anchors = container.querySelectorAll<HTMLElement>("[data-message-anchor]");
    const containerRect = container.getBoundingClientRect();
    const readingLine = containerRect.top + READING_LINE_OFFSET;

    let current: string | null = null;
    const visible: string[] = [];

    anchors.forEach((el) => {
      const id = el.dataset.messageAnchor;
      if (!id) return;
      const rect = el.getBoundingClientRect();
      if (rect.bottom >= containerRect.top && rect.top <= containerRect.bottom) {
        visible.push(id);
      }
      // Anchors are visited in document order, so the last one above the
      // reading line wins.
      if (rect.top <= readingLine) {
        current = id;
      }
    });

    // Scrolled above the first anchor: treat the first turn as current.
    if (current === null && anchors.length > 0) {
      current = anchors[0].dataset.messageAnchor ?? null;
    }

    setCurrentAnchorId((prev) => (prev === current ? prev : current));
    setVisibleAnchorIds((prev) => (sameOrder(prev, visible) ? prev : visible));
  }, [containerRef]);

  const schedule = useCallback(() => {
    if (frameRef.current !== null) return;
    frameRef.current = requestAnimationFrame(() => {
      frameRef.current = null;
      compute();
    });
  }, [compute]);

  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;

    container.addEventListener("scroll", schedule, { passive: true });
    window.addEventListener("resize", schedule);
    return () => {
      container.removeEventListener("scroll", schedule);
      window.removeEventListener("resize", schedule);
      if (frameRef.current !== null) {
        cancelAnimationFrame(frameRef.current);
        frameRef.current = null;
      }
    };
  }, [containerRef, schedule]);

  // Recompute when the transcript grows/shrinks or streams new content.
  useEffect(() => {
    schedule();
  }, [anchorCount, schedule]);

  const scrollToAnchor = useCallback(
    (id: string, behavior: ScrollBehavior = "smooth") => {
      const container = containerRef.current;
      if (!container) return;
      const el = container.querySelector<HTMLElement>(`[data-message-anchor="${CSS.escape(id)}"]`);
      el?.scrollIntoView({ behavior, block: "start" });
    },
    [containerRef],
  );

  return { currentAnchorId, visibleAnchorIds, scrollToAnchor };
}
