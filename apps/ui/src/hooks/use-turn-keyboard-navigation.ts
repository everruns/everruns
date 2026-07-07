// Keyboard turn stepping for the chat transcript.
//
// Builds on the navigation rail's `scrollToAnchor`: Alt+↑/↓ steps to the
// previous/next turn (works even while typing in the composer, since Alt+Arrow
// is not a text-editing shortcut), and bare j/k do the same when focus is not
// in an editable field.
//
// Decisions:
// - Stepping is driven by an explicit index ref that advances one turn per
//   keypress, so rapid presses and end-of-list jumps stay monotonic instead of
//   reading a not-yet-settled `currentAnchorId`.
// - The index re-syncs to `currentAnchorId` (the turn under the reading line)
//   whenever the user scrolls, except for a short window after our own jump —
//   otherwise the jump's own scroll would reset the index we just advanced.

import { useEffect, useRef } from "react";

// After a keyboard jump, ignore the resulting `currentAnchorId` change for this
// long so smooth-scroll intermediates and unreachable end-of-list turns don't
// reset the index we just advanced.
const SELF_NAV_SUPPRESS_MS = 500;

function isEditableTarget(target: EventTarget | null): boolean {
  if (!(target instanceof HTMLElement)) return false;
  const tag = target.tagName;
  return tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable;
}

interface UseTurnKeyboardNavigationOptions {
  /** Turn anchor ids in document order. */
  anchorIds: string[];
  /** Turn currently under the reading line (from the visibility hook). */
  currentAnchorId: string | null;
  /** Jump to a turn — typically the rail's `scrollToAnchor`. */
  onNavigate: (id: string) => void;
  enabled?: boolean;
}

export function useTurnKeyboardNavigation({
  anchorIds,
  currentAnchorId,
  onNavigate,
  enabled = true,
}: UseTurnKeyboardNavigationOptions) {
  const anchorIdsRef = useRef(anchorIds);
  anchorIdsRef.current = anchorIds;
  const onNavigateRef = useRef(onNavigate);
  onNavigateRef.current = onNavigate;

  // Index we treat as active for stepping. Advanced directly on each keypress.
  const activeIndexRef = useRef(-1);
  // Timestamp until which `currentAnchorId` echoes from our own jump are ignored.
  const suppressUntilRef = useRef(0);

  // Follow the reading anchor while the user scrolls, but not right after our
  // own jump (whose scroll would otherwise clobber the advanced index).
  useEffect(() => {
    if (Date.now() < suppressUntilRef.current) return;
    activeIndexRef.current = currentAnchorId ? anchorIdsRef.current.indexOf(currentAnchorId) : -1;
  }, [currentAnchorId]);

  useEffect(() => {
    if (!enabled) return;

    const handleKeyDown = (event: KeyboardEvent) => {
      const editable = isEditableTarget(event.target);
      const altStep =
        event.altKey &&
        !event.ctrlKey &&
        !event.metaKey &&
        (event.key === "ArrowDown" || event.key === "ArrowUp");
      const vimStep =
        !editable &&
        !event.altKey &&
        !event.ctrlKey &&
        !event.metaKey &&
        (event.key === "j" || event.key === "k");
      if (!altStep && !vimStep) return;

      const ids = anchorIdsRef.current;
      if (ids.length === 0) return;

      const direction = event.key === "ArrowDown" || event.key === "j" ? 1 : -1;
      const base = activeIndexRef.current;
      const nextIndex =
        base < 0
          ? direction > 0
            ? 0
            : ids.length - 1
          : Math.min(Math.max(base + direction, 0), ids.length - 1);

      // Recognized shortcut: swallow it so Alt+Arrow / j/k don't also scroll
      // the page or type into a focused control outside the composer.
      event.preventDefault();
      if (nextIndex === base) return;

      activeIndexRef.current = nextIndex;
      suppressUntilRef.current = Date.now() + SELF_NAV_SUPPRESS_MS;
      onNavigateRef.current(ids[nextIndex]);
    };

    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [enabled]);
}
