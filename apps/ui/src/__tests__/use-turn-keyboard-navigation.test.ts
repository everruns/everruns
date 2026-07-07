import { renderHook } from "@testing-library/react";
import { useTurnKeyboardNavigation } from "@/hooks/use-turn-keyboard-navigation";

const IDS = ["a", "b", "c", "d"];

function press(key: string, init: KeyboardEventInit = {}, target?: EventTarget) {
  const event = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
  (target ?? document).dispatchEvent(event);
  return event;
}

describe("useTurnKeyboardNavigation", () => {
  afterEach(() => {
    document.body.innerHTML = "";
  });

  it("steps to the next turn on Alt+ArrowDown from the reading anchor", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: IDS, currentAnchorId: "b", onNavigate }),
    );

    const event = press("ArrowDown", { altKey: true });

    expect(onNavigate).toHaveBeenCalledWith("c");
    expect(event.defaultPrevented).toBe(true);
  });

  it("steps to the previous turn on Alt+ArrowUp", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: IDS, currentAnchorId: "c", onNavigate }),
    );

    press("ArrowUp", { altKey: true });

    expect(onNavigate).toHaveBeenCalledWith("b");
  });

  it("advances monotonically on rapid presses without waiting for currentAnchorId", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: IDS, currentAnchorId: "a", onNavigate }),
    );

    press("ArrowDown", { altKey: true });
    press("ArrowDown", { altKey: true });

    expect(onNavigate.mock.calls.map((call) => call[0])).toEqual(["b", "c"]);
  });

  it("clamps at the last turn and does not navigate past it", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: IDS, currentAnchorId: "d", onNavigate }),
    );

    const event = press("ArrowDown", { altKey: true });

    expect(onNavigate).not.toHaveBeenCalled();
    // Still swallowed so it doesn't scroll the page.
    expect(event.defaultPrevented).toBe(true);
  });

  it("supports bare j/k when focus is not in an editable field", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: IDS, currentAnchorId: "b", onNavigate }),
    );

    press("j");
    expect(onNavigate).toHaveBeenLastCalledWith("c");
    press("k");
    expect(onNavigate).toHaveBeenLastCalledWith("b");
  });

  it("ignores bare j/k while typing in an input", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: IDS, currentAnchorId: "b", onNavigate }),
    );

    const input = document.createElement("input");
    document.body.appendChild(input);
    press("j", {}, input);

    expect(onNavigate).not.toHaveBeenCalled();
  });

  it("still allows Alt+Arrow while focus is in an input", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: IDS, currentAnchorId: "b", onNavigate }),
    );

    const textarea = document.createElement("textarea");
    document.body.appendChild(textarea);
    press("ArrowDown", { altKey: true }, textarea);

    expect(onNavigate).toHaveBeenCalledWith("c");
  });

  it("does nothing when there are no anchors", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({ anchorIds: [], currentAnchorId: null, onNavigate }),
    );

    press("ArrowDown", { altKey: true });

    expect(onNavigate).not.toHaveBeenCalled();
  });

  it("does not bind listeners when disabled", () => {
    const onNavigate = jest.fn();
    renderHook(() =>
      useTurnKeyboardNavigation({
        anchorIds: IDS,
        currentAnchorId: "b",
        onNavigate,
        enabled: false,
      }),
    );

    press("ArrowDown", { altKey: true });

    expect(onNavigate).not.toHaveBeenCalled();
  });
});
