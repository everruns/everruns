import { act, render } from "@testing-library/react";
import { useScrollManager } from "@/hooks/use-scroll-manager";

function Transcript({ ownSend, events }: { ownSend: string | null; events: number }) {
  const { scrollContainerRef, hasNewMessages } = useScrollManager({
    eventCount: events,
    eventsLoaded: true,
    hasMoreEvents: false,
    loadingOlderEvents: false,
    loadOlderEvents: () => {},
    sessionId: "session-1",
    latestOwnSendId: ownSend,
  });
  return (
    <div ref={scrollContainerRef} data-testid="scroller">
      {hasNewMessages ? "new" : "none"}
    </div>
  );
}

describe("useScrollManager", () => {
  beforeEach(() => {
    jest.spyOn(window, "requestAnimationFrame").mockImplementation((cb) => {
      cb(0);
      return 0;
    });
  });
  afterEach(() => jest.restoreAllMocks());

  it("scrolls to the reader's own message even after they scrolled up", () => {
    const scrollTo = jest.fn();
    Object.defineProperty(HTMLElement.prototype, "scrollTo", {
      configurable: true,
      value: scrollTo,
    });
    const { getByTestId, rerender } = render(<Transcript ownSend={null} events={0} />);
    const scroller = getByTestId("scroller");
    rerender(<Transcript ownSend={null} events={3} />);

    // Reader scrolls far up to re-read an earlier reply.
    Object.defineProperties(scroller, {
      scrollHeight: { configurable: true, value: 3000 },
      clientHeight: { configurable: true, value: 600 },
      scrollTop: { configurable: true, value: 100, writable: true },
    });
    act(() => {
      scroller.dispatchEvent(new Event("scroll"));
    });
    scrollTo.mockClear();

    // New stream content alone does not move them.
    rerender(<Transcript ownSend={null} events={4} />);
    expect(scrollTo.mock.calls).toEqual([]);
    expect(scroller).toHaveTextContent("new");

    // Their own send does, and clears the pill.
    rerender(<Transcript ownSend="client-1" events={5} />);
    expect(scrollTo).toHaveBeenCalledWith({ top: 3000, behavior: "smooth" });
    expect(scroller).toHaveTextContent("none");
  });
});
