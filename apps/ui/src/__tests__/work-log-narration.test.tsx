import { render, screen } from "@testing-library/react";
import * as words from "@pierre/words/react";

import { WorkLogNarration } from "@/components/chat/work-log-narration";

// jest maps @pierre/words to src/test-mocks/pierre-words.tsx, which records its props.
const markdownProps = () =>
  (words as unknown as { lastMarkdownProps: Record<string, unknown> }).lastMarkdownProps;

describe("WorkLogNarration", () => {
  it("passes links, emphasis, and line breaks through the shared Markdown pipeline", () => {
    const narration = "Created [Hourly Dad Jokes](/agents/agent_123).\nRuns every hour in **UTC**.";
    const { container } = render(<WorkLogNarration>{narration}</WorkLogNarration>);

    expect(screen.getByTestId("words-markdown")).toBeInTheDocument();
    expect(markdownProps().children).toBe(narration);
    expect(container.querySelector(".markdown-work-log")).toHaveClass("whitespace-pre-wrap");
    expect((markdownProps().components as Record<string, unknown>).a).toEqual(expect.any(Function));
  });

  it("renders unsafe links as plain text and allows no raw HTML", () => {
    render(<WorkLogNarration>{'Avoid [unsafe](javascript:alert("x")).'}</WorkLogNarration>);

    const link = (markdownProps().components as Record<string, (html: string) => string>).a;
    expect(link('<a href="javascript:alert(&quot;x&quot;)">unsafe</a>')).toBe(
      "<span>unsafe</span>",
    );
    expect(markdownProps().allowedHtml).toBeUndefined();
  });
});
