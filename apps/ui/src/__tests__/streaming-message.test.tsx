import { act, render, screen } from "@testing-library/react";
import { StreamingMessage } from "@/components/streaming-message";

jest.mock("@/components/chat/message-content", () => ({
  MessageContent: ({ text }: { text: string }) => <div data-testid="message-content">{text}</div>,
}));

function advanceFrame() {
  act(() => {
    jest.advanceTimersByTime(16);
  });
}

describe("StreamingMessage", () => {
  beforeEach(() => {
    jest.useFakeTimers();
  });

  afterEach(() => {
    jest.useRealTimers();
  });

  it("reveals accepted text incrementally instead of rendering a full chunk immediately", () => {
    render(<StreamingMessage text="Hello" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("H");

    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("He");

    advanceFrame();
    advanceFrame();
    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("Hello");
  });

  it("reveals grapheme clusters without splitting combined glyphs", () => {
    render(<StreamingMessage text="👩‍💻a" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("👩‍💻");

    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("👩‍💻a");
  });

  it("keeps typing from the visible prefix when a larger accumulated delta arrives", () => {
    const { rerender } = render(<StreamingMessage text="Hel" />);

    advanceFrame();
    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("Hel");

    rerender(<StreamingMessage text="Hello world" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("Hel");

    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("Hell");
  });

  it("resets cleanly when the target text is replaced by a different turn", () => {
    const { rerender } = render(<StreamingMessage text="Previous" />);

    act(() => {
      jest.runOnlyPendingTimers();
    });

    rerender(<StreamingMessage text="New" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("N");
  });
});
