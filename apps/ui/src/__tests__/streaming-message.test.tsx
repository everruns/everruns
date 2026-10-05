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
    render(<StreamingMessage messageId="message-1" text="Hello" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("H");

    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("He");

    advanceFrame();
    advanceFrame();
    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("Hello");
  });

  it("reveals grapheme clusters without splitting combined glyphs", () => {
    render(<StreamingMessage messageId="message-1" text="👩‍💻a" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("👩‍💻");

    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("👩‍💻a");
  });

  it("keeps typing from the visible prefix when a larger accumulated delta arrives", () => {
    const { rerender } = render(<StreamingMessage messageId="message-1" text="Hel" />);

    advanceFrame();
    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("Hel");

    rerender(<StreamingMessage messageId="message-1" text="Hello world" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("Hel");

    advanceFrame();
    expect(screen.getByTestId("message-content")).toHaveTextContent("Hello");
  });

  it("catches up with a large chunk within one delta batch", () => {
    const { rerender } = render(<StreamingMessage messageId="message-1" text="A" />);
    const chunk = "A" + "b".repeat(239);

    rerender(<StreamingMessage messageId="message-1" text={chunk} />);
    for (let frame = 0; frame < 6; frame++) advanceFrame();

    expect(screen.getByTestId("message-content")).toHaveTextContent(chunk);
  });

  it("resets cleanly when a different message starts in the same turn", () => {
    const { rerender } = render(
      <StreamingMessage messageId="commentary-message" text="Previous" />,
    );

    act(() => {
      jest.runOnlyPendingTimers();
    });

    rerender(<StreamingMessage messageId="final-message" text="Final" />);

    expect(screen.getByTestId("message-content")).toHaveTextContent("F");
    expect(screen.getByTestId("message-content")).not.toHaveTextContent("Previous");
  });
});
