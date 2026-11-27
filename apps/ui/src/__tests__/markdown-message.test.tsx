import { act, render, screen, waitFor } from "@testing-library/react";
import * as words from "@pierre/words/react";

jest.mock("pele", () => ({ render: () => ({ svg: "<svg></svg>" }) }));

import { MarkdownMessage } from "@/components/markdown/markdown-message";
import { alertBlockquoteHook, createLinkHook } from "@/components/markdown/markdown-hooks";
import { getMarkdownLinkKind } from "@/components/markdown/markdown-link";
import type { CitationSource } from "@/components/chat/message-citations";

// jest maps @pierre/words to src/test-mocks/pierre-words.tsx, which records its props.
const markdownProps = () =>
  (words as unknown as { lastMarkdownProps: Record<string, unknown> }).lastMarkdownProps;
const plugins = () => markdownProps().plugins as Record<string, { name: string } | undefined>;

describe("MarkdownMessage", () => {
  it("hands the text and the console's hooks to Words", () => {
    render(<MarkdownMessage isAnimating>Hello **world**</MarkdownMessage>);

    expect(screen.getByTestId("words-markdown")).toHaveTextContent("Hello **world**");
    expect(markdownProps().isAnimating).toBe(true);
    const components = markdownProps().components as Record<string, unknown>;
    expect(components.a).toEqual(expect.any(Function));
    expect(components.blockquote).toBe(alertBlockquoteHook);
  });

  it("keeps only the presentational HTML allowlist, and none with skipHtml", () => {
    const { rerender } = render(<MarkdownMessage>text</MarkdownMessage>);
    const allowed = markdownProps().allowedHtml as Record<string, readonly string[]>;
    expect(Object.keys(allowed).sort()).toEqual(["br", "details", "kbd", "sub", "summary", "sup"]);
    expect(allowed.details).toEqual(["open"]);

    rerender(<MarkdownMessage skipHtml>text</MarkdownMessage>);
    expect(markdownProps().allowedHtml).toBeUndefined();
  });

  it("loads no plugin for text without code or diagrams", () => {
    render(<MarkdownMessage>Just prose.</MarkdownMessage>);

    expect(plugins().code).toBeUndefined();
    expect(plugins().mermaid).toBeUndefined();
  });

  it("loads the highlighter once the text has a code fence", async () => {
    render(<MarkdownMessage>{"```ts\nconst a = 1;\n```"}</MarkdownMessage>);

    await waitFor(() => expect(plugins().code?.name).toBe("code"));
    expect(plugins().mermaid).toBeUndefined();
  });

  it("loads Pele once the text has a mermaid fence", async () => {
    render(
      <MarkdownMessage enableCodeHighlighting={false}>
        {"```mermaid\nflowchart LR\nA --> B\n```"}
      </MarkdownMessage>,
    );

    await waitFor(() => expect(plugins().mermaid?.name).toBe("mermaid"));
    expect(plugins().code).toBeUndefined();
  });

  it("mounts citation chips and link icons into the slots the hooks leave", async () => {
    const citations = new Map<number, CitationSource>([
      [1, { n: 1, uri: "https://example.com/a", title: "Example", origin: "web_search" }],
    ]);
    const { container } = render(<MarkdownMessage citations={citations}>text</MarkdownMessage>);
    const link = (markdownProps().components as Record<string, (html: string) => string>).a;

    // Stand in for Words writing the hooks' output into the container.
    const target = container.querySelector('[data-testid="words-markdown"]')!;
    await act(async () => {
      target.innerHTML = [
        link('<a href="#cite-1">1</a>'),
        link('<a href="https://github.com/everruns/everruns">repo</a>'),
      ].join(" ");
    });

    await waitFor(() =>
      expect(target.querySelector('[data-md-slot="cite"]')?.textContent).toBe("1"),
    );
    expect(
      target.querySelector('[data-md-slot="link-icon"] svg.markdown-link-icon'),
    ).not.toBeNull();
  });

  it("adds zoom controls to drawn diagrams", async () => {
    const { container } = render(<MarkdownMessage>diagram</MarkdownMessage>);
    const target = container.querySelector('[data-testid="words-markdown"]')!;
    await act(async () => {
      target.innerHTML =
        '<div data-markdown="code-block" data-language="mermaid"><div data-markdown="code-header"><span>mermaid</span><span></span></div><div data-markdown="mermaid"><svg class="pele" width="200"></svg></div></div>';
    });

    const zoomIn = await screen.findByRole("button", { name: "Zoom in" });
    const svg = target.querySelector("svg")!;
    await act(async () => zoomIn.click());
    expect(svg.style.width).toBe("300px");
    await act(async () => screen.getByRole("button", { name: "Zoom out" }).click());
    expect(svg.style.width).toBe("");
  });
});

describe("createLinkHook", () => {
  const link = createLinkHook();

  it("opens safe links in a new tab with a source-icon slot", () => {
    expect(link('<a href="https://github.com/everruns/everruns">repo</a>')).toBe(
      '<a href="https://github.com/everruns/everruns" target="_blank" rel="noopener noreferrer" class="markdown-link-with-icon"><span data-md-slot="link-icon" data-kind="github"></span>repo</a>',
    );
  });

  it("gives links without a known source no icon", () => {
    expect(link('<a href="mailto:hi@example.com">mail</a>')).toBe(
      '<a href="mailto:hi@example.com" target="_blank" rel="noopener noreferrer">mail</a>',
    );
  });

  it.each(["javascript:alert(&quot;x&quot;)", "java&#10;script:alert(1)", "data:text/html,x"])(
    "renders unsafe href %s as plain text",
    (href) => {
      expect(link(`<a href="${href}">bad</a>`)).toBe("<span>bad</span>");
    },
  );

  it("leaves in-page anchors such as footnotes as they are", () => {
    const footnote = '<a href="#fn-1" id="fnref-1">1</a>';
    expect(link(footnote)).toBe(footnote);
  });

  it("turns known citation markers into chip slots and drops unknown ones", () => {
    const cite = createLinkHook(new Set([2]));
    expect(cite('<a href="#cite-2">2</a>')).toBe('<span data-md-slot="cite" data-n="2"></span>');
    expect(cite('<a href="#cite-9">9</a>')).toBe("");
  });

  it("treats citation markers as anchors when the message has no citations", () => {
    expect(link('<a href="#cite-2">2</a>')).toBe('<a href="#cite-2">2</a>');
  });
});

describe("alertBlockquoteHook", () => {
  it("turns a [!WARNING] blockquote into a GitHub-style alert", () => {
    const html = alertBlockquoteHook(
      '<blockquote data-markdown="blockquote"><p>[!WARNING]\nMind the gap.</p></blockquote>',
    );

    expect(html).toMatch(/^<div class="markdown-alert markdown-alert-warning" dir="auto">/);
    expect(html).toContain('<p class="markdown-alert-title" dir="auto"><svg class="octicon"');
    expect(html).toContain("Warning</p><p>Mind the gap.</p></div>");
    expect(html).not.toContain("blockquote");
  });

  it("drops the empty paragraph left by a marker on its own line", () => {
    const html = alertBlockquoteHook("<blockquote><p>[!NOTE]</p><p>Body</p></blockquote>");

    expect(html).toContain("Note</p><p>Body</p></div>");
    expect(html).not.toContain("<p></p>");
  });

  it("leaves ordinary blockquotes alone", () => {
    const quote = "<blockquote><p>Just a quote</p></blockquote>";
    expect(alertBlockquoteHook(quote)).toBe(quote);
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
