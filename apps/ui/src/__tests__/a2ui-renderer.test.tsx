import { fireEvent, render, screen } from "@testing-library/react";
import { A2UIBlock } from "@/components/chat/a2ui-renderer";

// A2UI blocks render inside session-scoped pages; mock the session context and
// route params that useActionDispatch depends on so the component can render.
const mockMutate = jest.fn();

jest.mock("next/navigation", () => ({
  useParams: () => ({ sessionId: "session_test" }),
}));

jest.mock("@/app/(main)/sessions/[sessionId]/session-context", () => ({
  useSessionContext: () => ({ sendMessage: { mutate: mockMutate } }),
}));

function renderA2UI(node: unknown, isStreaming?: boolean) {
  return render(<A2UIBlock code={JSON.stringify(node)} isStreaming={isStreaming} />);
}

beforeEach(() => {
  mockMutate.mockClear();
});

describe("A2UIBlock structured rendering", () => {
  it("renders a Heading's text", () => {
    renderA2UI({ type: "Heading", props: { text: "Hello world" } });
    expect(screen.getByText("Hello world")).toBeInTheDocument();
  });

  it("renders a Badge label nested inside a Stack", () => {
    renderA2UI({
      type: "Stack",
      children: [{ type: "Badge", props: { label: "Beta" } }],
    });
    expect(screen.getByText("Beta")).toBeInTheDocument();
  });
});

describe("A2UIBlock URL allowlist (THREAT[TM-WEB-A2UI-01])", () => {
  it("renders an Image with a safe https src", () => {
    renderA2UI({
      type: "Image",
      props: { src: "https://example.com/a.png", alt: "pic" },
    });
    const img = screen.getByRole("img");
    expect(img).toHaveAttribute("src", "https://example.com/a.png");
  });

  it.each(["javascript:alert(1)", "data:text/html,<script>alert(1)</script>"])(
    "does not render an Image for unsafe src %s",
    (src) => {
      renderA2UI({ type: "Image", props: { src, alt: "x" } });
      expect(screen.queryByRole("img")).not.toBeInTheDocument();
    },
  );

  it("opens a window for a Button open_url action with a safe url", () => {
    const openSpy = jest.spyOn(window, "open").mockImplementation(() => null);
    renderA2UI({
      type: "Button",
      props: { label: "Open", action: { type: "open_url", url: "https://example.com" } },
    });

    fireEvent.click(screen.getByRole("button", { name: "Open" }));

    expect(openSpy).toHaveBeenCalledTimes(1);
    expect(openSpy).toHaveBeenCalledWith("https://example.com", "_blank", "noopener,noreferrer");
    openSpy.mockRestore();
  });

  it("does not open a window for a Button open_url action with an unsafe url", () => {
    const openSpy = jest.spyOn(window, "open").mockImplementation(() => null);
    renderA2UI({
      type: "Button",
      props: { label: "Bad", action: { type: "open_url", url: "javascript:alert(1)" } },
    });

    fireEvent.click(screen.getByRole("button", { name: "Bad" }));

    expect(openSpy).not.toHaveBeenCalled();
    openSpy.mockRestore();
  });
});

describe("A2UIBlock parse fallbacks", () => {
  it("shows the raw code when JSON is unrecoverable and not streaming", () => {
    render(<A2UIBlock code="not json at all" />);
    expect(screen.getByText("not json at all")).toBeInTheDocument();
  });

  it("shows a placeholder (not raw code) when unrecoverable JSON is still streaming", () => {
    render(<A2UIBlock code="not json at all" isStreaming />);
    expect(screen.getByText("…")).toBeInTheDocument();
    expect(screen.queryByText("not json at all")).not.toBeInTheDocument();
  });

  it("renders nothing for empty code", () => {
    const { container } = render(<A2UIBlock code="" />);
    expect(container).toBeEmptyDOMElement();
  });
});
