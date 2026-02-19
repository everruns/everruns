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

jest.mock("remark-github-blockquote-alert", () => jest.fn());

import { StreamdownMessage, InlineStreamdownMessage } from "@/components/chat/streamdown-message";

describe("StreamdownMessage", () => {
  beforeEach(() => {
    capturedStreamdownProps = {};
  });

  it("passes components with ExternalLink override for anchor tags", () => {
    render(<StreamdownMessage>Hello **world**</StreamdownMessage>);

    expect(capturedStreamdownProps.components).toBeDefined();
    const components = capturedStreamdownProps.components as Record<string, unknown>;
    expect(components.a).toBeDefined();
  });

  it("ExternalLink component renders links with target=_blank", () => {
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
  });

  it("ExternalLink preserves additional attributes", () => {
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

    expect(link).toHaveAttribute("class", "custom-link");
    expect(link).toHaveAttribute("title", "Example");
    expect(link).toHaveAttribute("target", "_blank");
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

  it("enables code highlighting by default", () => {
    render(<StreamdownMessage>code block</StreamdownMessage>);

    const plugins = capturedStreamdownProps.plugins as Record<string, unknown>;
    expect(plugins.code).toEqual({ name: "mock-code-plugin" });
  });

  it("disables code highlighting when prop is false", () => {
    render(<StreamdownMessage enableCodeHighlighting={false}>code block</StreamdownMessage>);

    const plugins = capturedStreamdownProps.plugins as Record<string, unknown>;
    expect(plugins.code).toBeUndefined();
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

  it("inherits ExternalLink component for links", () => {
    render(<InlineStreamdownMessage>Content</InlineStreamdownMessage>);

    const components = capturedStreamdownProps.components as Record<string, unknown>;
    expect(components.a).toBeDefined();
  });
});
