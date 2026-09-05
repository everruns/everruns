import { render, screen, act } from "@testing-library/react";

// Capture props passed to Streamdown to verify components/plugins
let capturedStreamdownProps: Record<string, unknown> = {};

jest.mock("streamdown", () => ({
  Streamdown: (props: Record<string, unknown>) => {
    capturedStreamdownProps = props;
    // Render children as text so we can assert content
    return <div data-testid="streamdown-mock">{props.children as string}</div>;
  },
}));

jest.mock("@streamdown/code", () => ({
  code: { name: "mock-code-plugin" },
}));

const mockCreateMermaidPlugin = jest.fn(() => ({
  name: "mermaid",
  type: "diagram",
  language: "mermaid",
  getMermaid: jest.fn(),
}));

jest.mock("@streamdown/mermaid", () => ({
  createMermaidPlugin: mockCreateMermaidPlugin,
  mermaid: { name: "mermaid", type: "diagram", language: "mermaid", getMermaid: jest.fn() },
}));

jest.mock("remark-gfm", () => jest.fn());
jest.mock("remark-github-blockquote-alert", () => jest.fn());

import { StreamdownMessage, InlineStreamdownMessage } from "@/components/chat/streamdown-message";
import { getMarkdownLinkKind, MarkdownLink } from "@/components/markdown/markdown-link";

describe("StreamdownMessage", () => {
  beforeEach(() => {
    capturedStreamdownProps = {};
  });

  it("MarkdownLink component renders links with target=_blank", () => {
    render(<StreamdownMessage>test</StreamdownMessage>);

    const components = capturedStreamdownProps.components as Record<
      string,
      React.ComponentType<React.AnchorHTMLAttributes<HTMLAnchorElement>>
    >;
    const AnchorComponent = components.a;

    // Render the anchor component directly
    const { container } = render(
      <AnchorComponent href="https://example.com">Click me</AnchorComponent>,
    );
    const link = container.querySelector("a")!;

    expect(link).toHaveAttribute("target", "_blank");
    expect(link).toHaveAttribute("rel", "noopener noreferrer");
    expect(link).toHaveAttribute("href", "https://example.com");
    expect(link).toHaveTextContent("Click me");
    expect(link.querySelector("svg")).not.toBeNull();
  });

  it("MarkdownLink preserves additional attributes", () => {
    render(<StreamdownMessage>test</StreamdownMessage>);

    const components = capturedStreamdownProps.components as Record<
      string,
      React.ComponentType<React.AnchorHTMLAttributes<HTMLAnchorElement>>
    >;
    const AnchorComponent = components.a;

    const { container } = render(
      <AnchorComponent href="https://example.com" className="custom-link" title="Example">
        Link
      </AnchorComponent>,
    );
    const link = container.querySelector("a")!;

    expect(link).toHaveClass("custom-link");
    expect(link).toHaveAttribute("title", "Example");
    expect(link).toHaveAttribute("target", "_blank");
  });

  it.each(['javascript:alert("x")', "java\nscript:alert(1)", "data:text/html,<script />"])(
    "MarkdownLink renders unsafe URL %s as inert text",
    (href) => {
      const { container } = render(<MarkdownLink href={href}>Unsafe destination</MarkdownLink>);

      expect(screen.getByText("Unsafe destination")).toBeInTheDocument();
      expect(container.querySelector("a")).toBeNull();
    },
  );

  it("MarkdownLink renders Everruns logo for internal links", () => {
    render(<StreamdownMessage>test</StreamdownMessage>);

    const components = capturedStreamdownProps.components as Record<
      string,
      React.ComponentType<React.AnchorHTMLAttributes<HTMLAnchorElement>>
    >;
    const AnchorComponent = components.a;

    const { container } = render(<AnchorComponent href="/sessions/123">Session</AnchorComponent>);
    const logo = container.querySelector(".markdown-link-everruns-icon");

    expect(logo).toHaveAttribute("aria-hidden", "true");
  });

  it("renders with default variant (bg-muted p-4)", () => {
    const { container } = render(<StreamdownMessage>Content</StreamdownMessage>);

    const wrapper = container.querySelector(".streamdown");
    expect(wrapper?.className).toContain("bg-muted");
    expect(wrapper?.className).toContain("p-4");
  });

  it("renders inline variant without background", () => {
    const { container } = render(<StreamdownMessage variant="inline">Content</StreamdownMessage>);

    const wrapper = container.querySelector(".streamdown");
    expect(wrapper?.className).toContain("bg-transparent");
    expect(wrapper?.className).not.toContain("bg-muted");
  });

  it("passes children to Streamdown", () => {
    render(<StreamdownMessage>Hello markdown</StreamdownMessage>);

    expect(screen.getByTestId("streamdown-mock")).toHaveTextContent("Hello markdown");
  });

  it("enables mermaid plugin after lazy load", async () => {
    await act(async () => {
      render(<StreamdownMessage>code block</StreamdownMessage>);
      // Wait for the lazy-loaded mermaid plugin to resolve
      await import("@streamdown/mermaid");
    });

    const plugins = capturedStreamdownProps.plugins as Record<string, unknown>;
    expect(plugins.mermaid).toMatchObject({ name: "mermaid", type: "diagram" });
    expect(mockCreateMermaidPlugin).toHaveBeenCalledWith({
      config: {
        securityLevel: "strict",
      },
    });
  });

  it("enables code highlighting by default", () => {
    render(<StreamdownMessage>code block</StreamdownMessage>);

    const plugins = capturedStreamdownProps.plugins as Record<string, unknown>;
    expect(plugins.code).toMatchObject({ name: "mock-code-plugin" });
  });

  it("disables code highlighting when prop is false", () => {
    render(<StreamdownMessage enableCodeHighlighting={false}>code block</StreamdownMessage>);

    const plugins = capturedStreamdownProps.plugins as Record<string, unknown>;
    expect(plugins.code).toBeUndefined();
  });

  it("enables GFM parsing so plain URLs render as links", () => {
    render(<StreamdownMessage>UI link: http://localhost:3000</StreamdownMessage>);

    const remarkPlugins = capturedStreamdownProps.remarkPlugins as unknown[];
    expect(remarkPlugins).toHaveLength(2);
  });
});

describe("InlineStreamdownMessage", () => {
  beforeEach(() => {
    capturedStreamdownProps = {};
  });

  it("renders with inline variant (no background)", () => {
    const { container } = render(
      <InlineStreamdownMessage>Instructions here</InlineStreamdownMessage>,
    );

    const wrapper = container.querySelector(".streamdown");
    expect(wrapper?.className).toContain("bg-transparent");
  });

  it("disables code highlighting", () => {
    render(<InlineStreamdownMessage>Instructions</InlineStreamdownMessage>);

    const plugins = capturedStreamdownProps.plugins as Record<string, unknown>;
    expect(plugins.code).toBeUndefined();
  });

  it("passes custom className", () => {
    const { container } = render(
      <InlineStreamdownMessage className="text-sm text-muted-foreground">
        Content
      </InlineStreamdownMessage>,
    );

    const wrapper = container.querySelector(".streamdown");
    expect(wrapper?.className).toContain("text-sm");
    expect(wrapper?.className).toContain("text-muted-foreground");
  });

  it("inherits MarkdownLink component for links", () => {
    render(<InlineStreamdownMessage>Content</InlineStreamdownMessage>);

    const components = capturedStreamdownProps.components as Record<string, unknown>;
    expect(components.a).toBe(MarkdownLink);
  });
});

describe("getMarkdownLinkKind", () => {
  it.each([
    ["https://github.com/everruns/everruns/pull/44", "github"],
    ["https://twitter.com/everruns_ai", "twitter"],
    ["https://x.com/everruns_ai", "twitter"],
    ["https://docs.everruns.com/api", "everruns"],
    ["/sessions/session_123", "everruns"],
    ["docs/setup.md", "everruns"],
    ["https://example.com/docs", "web"],
    ["#local-section", null],
    ["mailto:hello@everruns.com", null],
  ] as const)("classifies %s as %s", (href, expected) => {
    expect(getMarkdownLinkKind(href)).toBe(expected);
  });
});
