import { render, screen } from "@testing-library/react";

let capturedStreamdownProps: Record<string, unknown> = {};

jest.mock("streamdown", () => ({
  Streamdown: (props: Record<string, unknown>) => {
    capturedStreamdownProps = props;
    return <div data-testid="streamdown-mock">{props.children as string}</div>;
  },
}));
jest.mock("@streamdown/code", () => ({ code: { name: "mock-code-plugin" } }));
jest.mock("@streamdown/mermaid", () => ({
  createMermaidPlugin: jest.fn(() => ({ name: "mermaid", type: "diagram" })),
}));
jest.mock("remark-gfm", () => jest.fn());
jest.mock("remark-github-blockquote-alert", () => jest.fn());

import { WorkLogNarration } from "@/components/chat/work-log-narration";

describe("WorkLogNarration", () => {
  beforeEach(() => {
    capturedStreamdownProps = {};
  });

  it("passes links, emphasis, and line breaks through the shared Markdown pipeline", () => {
    const narration = "Created [Hourly Dad Jokes](/agents/agent_123).\nRuns every hour in **UTC**.";
    const { container } = render(<WorkLogNarration>{narration}</WorkLogNarration>);

    expect(screen.getByTestId("streamdown-mock")).toBeInTheDocument();
    expect(capturedStreamdownProps.children).toBe(narration);
    expect(container.querySelector(".streamdown-work-log")).toHaveClass("whitespace-pre-wrap");

    const components = capturedStreamdownProps.components as Record<string, unknown>;
    expect(components.a).toBeDefined();
  });

  it("disables raw HTML parsing while retaining Streamdown URL sanitization", () => {
    render(
      <WorkLogNarration>
        {'Avoid [unsafe](javascript:alert("x")) and <img src=x onerror="alert(1)">.'}
      </WorkLogNarration>,
    );

    expect(capturedStreamdownProps.skipHtml).toBe(true);
    expect(capturedStreamdownProps.urlTransform).toBeUndefined();
  });
});
