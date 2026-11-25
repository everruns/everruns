import { render, screen } from "@testing-library/react";

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

jest.mock("pele", () => ({ render: () => ({ svg: "<svg></svg>" }) }));

jest.mock("remark-gfm", () => jest.fn());
jest.mock("remark-github-blockquote-alert", () => jest.fn());

import { StreamdownMessage } from "@/components/chat/streamdown-message";
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

    const { container } = render(
      <AnchorComponent href="/no-such-page/123">Somewhere</AnchorComponent>,
    );
    const logo = container.querySelector(".markdown-link-everruns-icon");

    expect(logo).toHaveAttribute("aria-hidden", "true");
  });

  it("MarkdownLink uses the destination page icon for in-app links", () => {
    render(<StreamdownMessage>test</StreamdownMessage>);

    const components = capturedStreamdownProps.components as Record<
      string,
      React.ComponentType<React.AnchorHTMLAttributes<HTMLAnchorElement>>
    >;
    const AnchorComponent = components.a;

    const { container } = render(<AnchorComponent href="/harnesses/abc">Harness</AnchorComponent>);

    expect(container.querySelector(".markdown-link-everruns-icon")).toBeNull();
    expect(container.querySelector("svg.markdown-link-icon")).toHaveAttribute(
      "aria-hidden",
      "true",
    );
  });

  it("renders diagrams through Pele", async () => {
    render(<StreamdownMessage>code block</StreamdownMessage>);

    const plugins = capturedStreamdownProps.plugins as Record<
      string,
      { getMermaid: () => { render: (id: string, source: string) => Promise<{ svg: string }> } }
    >;
    expect(plugins.mermaid).toMatchObject({ name: "mermaid", type: "diagram" });
    const result = await plugins.mermaid.getMermaid().render("test", "flowchart LR\nA --> B");
    expect(result.svg).toContain("<svg");
  });

  it("disables code highlighting when prop is false", () => {
    render(<StreamdownMessage enableCodeHighlighting={false}>code block</StreamdownMessage>);

    const plugins = capturedStreamdownProps.plugins as Record<string, unknown>;
    expect(plugins.code).toBeUndefined();
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
