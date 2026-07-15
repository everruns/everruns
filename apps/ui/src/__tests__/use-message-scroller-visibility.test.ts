import { renderHook } from "@testing-library/react";
import type { RefObject } from "react";
import { useMessageScrollerVisibility } from "@/hooks/use-message-scroller-visibility";

// jsdom has no layout engine, so getBoundingClientRect is mocked per element to
// drive the hook's geometry math deterministically.
function rect(top: number, height: number): DOMRect {
  return {
    top,
    bottom: top + height,
    left: 0,
    right: 0,
    width: 0,
    height,
    x: 0,
    y: top,
    toJSON: () => ({}),
  } as DOMRect;
}

function makeContainer(): HTMLDivElement {
  const container = document.createElement("div");
  // Viewport spans 0..500 in client coordinates.
  container.getBoundingClientRect = () => rect(0, 500);
  document.body.appendChild(container);
  return container;
}

function addAnchor(
  container: HTMLElement,
  id: string,
  top: number,
  height: number,
): HTMLDivElement {
  const el = document.createElement("div");
  el.dataset.messageAnchor = id;
  el.getBoundingClientRect = () => rect(top, height);
  el.scrollIntoView = jest.fn();
  container.appendChild(el);
  return el;
}

describe("useMessageScrollerVisibility", () => {
  beforeAll(() => {
    if (typeof (globalThis as { CSS?: unknown }).CSS === "undefined") {
      (globalThis as { CSS?: unknown }).CSS = { escape: (value: string) => value };
    }
  });

  beforeEach(() => {
    jest.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });
    jest.spyOn(window, "cancelAnimationFrame").mockImplementation(() => {});
  });

  afterEach(() => {
    jest.restoreAllMocks();
    document.body.innerHTML = "";
  });

  it("marks the last anchor above the reading line as current", () => {
    const container = makeContainer();
    addAnchor(container, "a", -100, 50); // scrolled fully above the viewport
    addAnchor(container, "b", 10, 200); // above the reading line, on screen
    addAnchor(container, "c", 300, 100); // below the reading line, on screen
    const ref = { current: container } as RefObject<HTMLElement | null>;

    const { result } = renderHook(() => useMessageScrollerVisibility(ref, 3));

    expect(result.current.currentAnchorId).toBe("b");
    expect(result.current.visibleAnchorIds).toEqual(["b", "c"]);
  });

  it("falls back to the first anchor when scrolled above all of them", () => {
    const container = makeContainer();
    addAnchor(container, "a", 100, 200); // both below the reading line (top of list)
    addAnchor(container, "b", 340, 200);
    const ref = { current: container } as RefObject<HTMLElement | null>;

    const { result } = renderHook(() => useMessageScrollerVisibility(ref, 2));

    expect(result.current.currentAnchorId).toBe("a");
  });

  it("scrolls the matching anchor into view", () => {
    const container = makeContainer();
    const target = addAnchor(container, "b", 300, 100);
    const ref = { current: container } as RefObject<HTMLElement | null>;

    const { result } = renderHook(() => useMessageScrollerVisibility(ref, 1));
    result.current.scrollToAnchor("b");

    expect(target.scrollIntoView).toHaveBeenCalledWith({ behavior: "smooth", block: "start" });
  });
});
