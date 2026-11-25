"use client";

/**
 * MarkdownSlots - mounts React decorations into the HTML Words renders.
 *
 * Decisions:
 * - Words owns the DOM under its container and rewrites the still-streaming tail with
 *   innerHTML, so React cannot render inside it directly. The link hook leaves empty
 *   `data-md-slot` placeholders instead; this component watches the container and portals
 *   a React node into each one. A placeholder Words replaces is dropped with its portal.
 * - Portals render into nodes Words created and React never removes them itself: when a
 *   slot leaves the DOM its portal unmounts into the detached node, which is harmless.
 */

import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useUiTranslations } from "@/providers/locale-provider";
import { CitationChip, type CitationSource } from "@/components/chat/message-citations";
import { LinkIcon, type MarkdownLinkKind } from "@/components/markdown/markdown-link";

const LINK_KINDS = new Set<MarkdownLinkKind>(["everruns", "github", "twitter", "web"]);

let nextSlotId = 0;
const slotIds = new WeakMap<Element, number>();
function slotId(el: Element): number {
  let id = slotIds.get(el);
  if (id === undefined) {
    id = ++nextSlotId;
    slotIds.set(el, id);
  }
  return id;
}

function sameSlots(a: readonly Element[], b: readonly Element[]): boolean {
  return a.length === b.length && a.every((el, i) => el === b[i]);
}

export function MarkdownSlots({
  container,
  citations,
}: {
  container: HTMLElement | null;
  citations?: Map<number, CitationSource>;
}) {
  const [slots, setSlots] = useState<Element[]>([]);
  const [diagrams, setDiagrams] = useState<Element[]>([]);

  useEffect(() => {
    if (!container) return;
    const scan = () => {
      const found = Array.from(container.querySelectorAll("[data-md-slot]"));
      setSlots((current) => (sameSlots(current, found) ? current : found));
      // Zoom controls go into the header of each code block that holds a drawn diagram.
      const headers = Array.from(
        container.querySelectorAll(
          '[data-markdown="code-block"]:has([data-markdown="mermaid"] svg)',
        ),
        (block) => block.querySelector('[data-markdown="code-header"] > span:last-child'),
      ).filter((el): el is Element => el !== null);
      setDiagrams((current) => (sameSlots(current, headers) ? current : headers));
    };
    scan();
    const observer = new MutationObserver(scan);
    observer.observe(container, { childList: true, subtree: true });
    return () => observer.disconnect();
  }, [container]);

  return (
    <>
      {slots.map((slot) => {
        const node = renderSlot(slot, citations);
        return node ? createPortal(node, slot, String(slotId(slot))) : null;
      })}
      {diagrams.map((actions) =>
        createPortal(<DiagramZoom actions={actions} />, actions, `zoom-${slotId(actions)}`),
      )}
    </>
  );
}

const ZOOM_STEPS = [1, 1.5, 2, 3, 4];

/**
 * Zoom for a Pele diagram. The SVG scales to fit its column, so a wide flowchart can end up
 * too small to read; zooming widens it and the diagram area scrolls.
 */
function DiagramZoom({ actions }: { actions: Element }) {
  const ui = useUiTranslations();
  const [step, setStep] = useState(0);

  useEffect(() => {
    const svg = actions
      .closest('[data-markdown="code-block"]')
      ?.querySelector<SVGSVGElement>('[data-markdown="mermaid"] svg');
    if (!svg) return;
    const scale = ZOOM_STEPS[step];
    if (scale === 1) {
      svg.style.removeProperty("width");
      svg.style.removeProperty("max-width");
      return;
    }
    const natural = Number(svg.getAttribute("width")) || svg.getBoundingClientRect().width;
    svg.style.maxWidth = "none";
    svg.style.width = `${natural * scale}px`;
  }, [actions, step]);

  return (
    <>
      <button
        type="button"
        aria-label={ui("Zoom out")}
        disabled={step === 0}
        onClick={() => setStep((s) => Math.max(0, s - 1))}
      >
        −
      </button>
      <button
        type="button"
        aria-label={ui("Zoom in")}
        disabled={step === ZOOM_STEPS.length - 1}
        onClick={() => setStep((s) => Math.min(ZOOM_STEPS.length - 1, s + 1))}
      >
        +
      </button>
    </>
  );
}

function renderSlot(slot: Element, citations?: Map<number, CitationSource>) {
  const type = slot.getAttribute("data-md-slot");
  if (type === "cite") {
    const source = citations?.get(Number(slot.getAttribute("data-n")));
    return source ? <CitationChip source={source} /> : null;
  }
  if (type === "link-icon") {
    const kind = slot.getAttribute("data-kind") as MarkdownLinkKind | null;
    const href = slot.closest("a")?.getAttribute("href");
    return kind && LINK_KINDS.has(kind) && href ? <LinkIcon kind={kind} href={href} /> : null;
  }
  return null;
}
