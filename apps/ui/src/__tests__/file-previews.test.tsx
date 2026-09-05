import { MarkdownLink } from "@/components/markdown/markdown-link";
import { render, screen } from "@testing-library/react";

let capturedStreamdownProps: Record<string, unknown> = {};

// Mock streamdown to avoid ESM issues in Jest
jest.mock("streamdown", () => ({
  Streamdown: (props: Record<string, unknown>) => {
    capturedStreamdownProps = props;
    return <pre data-testid="streamdown-mock">{props.children as string}</pre>;
  },
}));

jest.mock("@streamdown/code", () => ({
  code: {},
}));

import {
  getPreviewType,
  canPreview,
  CSVPreview,
  ImagePreview,
  MarkdownPreview,
  SVGPreview,
  HtmlPreview,
  PdfPreview,
  parseFrontmatter,
} from "@/components/files/file-previews";

// ============================================
// getPreviewType Tests
// ============================================

describe("getPreviewType", () => {
  describe("code files", () => {
    it("returns 'code' for TypeScript files", () => {
      expect(getPreviewType("ts", "text")).toBe("code");
      expect(getPreviewType("tsx", "text")).toBe("code");
    });

    it("returns 'code' for JavaScript files", () => {
      expect(getPreviewType("js", "text")).toBe("code");
      expect(getPreviewType("jsx", "text")).toBe("code");
    });

    it("returns 'code' for Python files", () => {
      expect(getPreviewType("py", "text")).toBe("code");
    });

    it("returns 'code' for Rust files", () => {
      expect(getPreviewType("rs", "text")).toBe("code");
    });

    it("returns 'code' for Go files", () => {
      expect(getPreviewType("go", "text")).toBe("code");
    });

    it("returns 'code' for shell scripts", () => {
      expect(getPreviewType("sh", "text")).toBe("code");
      expect(getPreviewType("bash", "text")).toBe("code");
      expect(getPreviewType("zsh", "text")).toBe("code");
    });

    it("returns 'code' for config files", () => {
      expect(getPreviewType("yml", "text")).toBe("code");
      expect(getPreviewType("yaml", "text")).toBe("code");
      expect(getPreviewType("toml", "text")).toBe("code");
    });

    it("returns 'code' for web files", () => {
      expect(getPreviewType("css", "text")).toBe("code");
      expect(getPreviewType("scss", "text")).toBe("code");
    });
  });

  describe("html files", () => {
    it("returns 'html' for .html and .htm files (rendered preview path)", () => {
      expect(getPreviewType("html", "text")).toBe("html");
      expect(getPreviewType("htm", "text")).toBe("html");
    });
  });

  describe("pdf files", () => {
    it("returns 'pdf' for PDF files with base64 encoding", () => {
      expect(getPreviewType("pdf", "base64")).toBe("pdf");
    });
  });

  describe("data files", () => {
    it("returns 'csv' for CSV files", () => {
      expect(getPreviewType("csv", "text")).toBe("csv");
    });

    it("returns 'json' for JSON files", () => {
      expect(getPreviewType("json", "text")).toBe("json");
    });
  });

  describe("markdown files", () => {
    it("returns 'markdown' for .md files", () => {
      expect(getPreviewType("md", "text")).toBe("markdown");
    });

    it("returns 'markdown' for .markdown files", () => {
      expect(getPreviewType("markdown", "text")).toBe("markdown");
    });
  });

  describe("image files", () => {
    it("returns 'image' for PNG files with base64 encoding", () => {
      expect(getPreviewType("png", "base64")).toBe("image");
    });

    it("returns 'image' for JPEG files with base64 encoding", () => {
      expect(getPreviewType("jpg", "base64")).toBe("image");
      expect(getPreviewType("jpeg", "base64")).toBe("image");
    });

    it("returns 'image' for GIF files with base64 encoding", () => {
      expect(getPreviewType("gif", "base64")).toBe("image");
    });

    it("returns 'image' for WebP files with base64 encoding", () => {
      expect(getPreviewType("webp", "base64")).toBe("image");
    });

    it("returns 'svg' for SVG files with base64 encoding", () => {
      expect(getPreviewType("svg", "base64")).toBe("svg");
    });

    it("returns 'svg' for SVG files with text encoding", () => {
      expect(getPreviewType("svg", "text")).toBe("svg");
    });
  });

  describe("binary files", () => {
    it("returns 'binary' for unknown base64 files", () => {
      expect(getPreviewType("exe", "base64")).toBe("binary");
      expect(getPreviewType("bin", "base64")).toBe("binary");
      expect(getPreviewType("dll", "base64")).toBe("binary");
    });
  });

  describe("text files", () => {
    it("returns 'text' for plain text files", () => {
      expect(getPreviewType("txt", "text")).toBe("text");
    });

    it("returns 'text' for unknown text files", () => {
      expect(getPreviewType("xyz", "text")).toBe("text");
      expect(getPreviewType("unknown", "text")).toBe("text");
    });
  });

  describe("case insensitivity", () => {
    it("handles uppercase extensions", () => {
      expect(getPreviewType("TS", "text")).toBe("code");
      expect(getPreviewType("JSON", "text")).toBe("json");
      expect(getPreviewType("MD", "text")).toBe("markdown");
      expect(getPreviewType("PNG", "base64")).toBe("image");
    });

    it("handles mixed case extensions", () => {
      expect(getPreviewType("Ts", "text")).toBe("code");
      expect(getPreviewType("Json", "text")).toBe("json");
    });
  });
});

// ============================================
// canPreview Tests
// ============================================

describe("canPreview", () => {
  describe("previewable files", () => {
    it("returns true for code files", () => {
      expect(canPreview("ts", "text")).toBe(true);
      expect(canPreview("py", "text")).toBe(true);
      expect(canPreview("rs", "text")).toBe(true);
    });

    it("returns true for data files", () => {
      expect(canPreview("csv", "text")).toBe(true);
      expect(canPreview("json", "text")).toBe(true);
    });

    it("returns true for markdown files", () => {
      expect(canPreview("md", "text")).toBe(true);
    });

    it("returns true for image files", () => {
      expect(canPreview("png", "base64")).toBe(true);
      expect(canPreview("jpg", "base64")).toBe(true);
    });

    it("returns true for html files (sandboxed render path)", () => {
      expect(canPreview("html", "text")).toBe(true);
      expect(canPreview("htm", "text")).toBe(true);
    });

    it("returns true for pdf files (data: iframe path)", () => {
      expect(canPreview("pdf", "base64")).toBe(true);
    });
  });

  describe("non-previewable files", () => {
    it("returns false for plain text files", () => {
      expect(canPreview("txt", "text")).toBe(false);
    });

    it("returns false for binary files", () => {
      expect(canPreview("exe", "base64")).toBe(false);
      expect(canPreview("bin", "base64")).toBe(false);
    });

    it("returns true for SVG files (sandboxed preview path)", () => {
      expect(canPreview("svg", "base64")).toBe(true);
      expect(canPreview("svg", "text")).toBe(true);
    });

    it("returns false for unknown file types", () => {
      expect(canPreview("xyz", "text")).toBe(false);
    });
  });
});

// ============================================
// CSVPreview Tests
// ============================================

describe("CSVPreview", () => {
  describe("rendering", () => {
    it("renders table headers", () => {
      const csv = "name,age,city\nAlice,30,NYC";
      render(<CSVPreview content={csv} />);

      expect(screen.getByText("name")).toBeInTheDocument();
      expect(screen.getByText("age")).toBeInTheDocument();
      expect(screen.getByText("city")).toBeInTheDocument();
    });

    it("renders table data rows", () => {
      const csv = "name,age\nAlice,30\nBob,25";
      render(<CSVPreview content={csv} />);

      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.getByText("30")).toBeInTheDocument();
      expect(screen.getByText("Bob")).toBeInTheDocument();
      expect(screen.getByText("25")).toBeInTheDocument();
    });

    it("shows row and column count", () => {
      const csv = "a,b,c\n1,2,3\n4,5,6";
      render(<CSVPreview content={csv} />);

      expect(screen.getByText("2 rows, 3 columns")).toBeInTheDocument();
    });
  });

  describe("empty state", () => {
    it("shows empty message for empty content", () => {
      render(<CSVPreview content="" />);

      expect(screen.getByText("Empty or invalid CSV")).toBeInTheDocument();
    });
  });

  describe("quoted fields", () => {
    it("handles quoted fields with commas", () => {
      const csv = 'name,address\nJohn,"123 Main St, Apt 4"';
      render(<CSVPreview content={csv} />);

      expect(screen.getByText("John")).toBeInTheDocument();
      expect(screen.getByText("123 Main St, Apt 4")).toBeInTheDocument();
    });

    it("handles escaped quotes in fields", () => {
      const csv = 'name,quote\nAlice,"She said ""hello"""';
      render(<CSVPreview content={csv} />);

      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.getByText('She said "hello"')).toBeInTheDocument();
    });
  });

  describe("single column", () => {
    it("renders single column CSV", () => {
      const csv = "names\nAlice\nBob\nCharlie";
      render(<CSVPreview content={csv} />);

      expect(screen.getByText("names")).toBeInTheDocument();
      expect(screen.getByText("Alice")).toBeInTheDocument();
      expect(screen.getByText("Bob")).toBeInTheDocument();
      expect(screen.getByText("Charlie")).toBeInTheDocument();
      expect(screen.getByText("3 rows, 1 columns")).toBeInTheDocument();
    });
  });
});

// ============================================
// ImagePreview Tests
// ============================================

describe("ImagePreview", () => {
  // Base64 encoded 1x1 red pixel PNG
  const sampleBase64 =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

  describe("rendering", () => {
    it("renders image element", () => {
      render(<ImagePreview content={sampleBase64} extension="png" fileName="test.png" />);

      const img = screen.getByRole("img");
      expect(img).toBeInTheDocument();
    });

    it("sets correct alt text", () => {
      render(<ImagePreview content={sampleBase64} extension="png" fileName="test.png" />);

      const img = screen.getByRole("img");
      expect(img).toHaveAttribute("alt", "test.png");
    });

    it("sets correct data URL for PNG", () => {
      render(<ImagePreview content={sampleBase64} extension="png" fileName="test.png" />);

      const img = screen.getByRole("img");
      expect(img).toHaveAttribute("src", `data:image/png;base64,${sampleBase64}`);
    });

    it("sets correct data URL for JPEG", () => {
      render(<ImagePreview content={sampleBase64} extension="jpg" fileName="test.jpg" />);

      const img = screen.getByRole("img");
      expect(img).toHaveAttribute("src", `data:image/jpeg;base64,${sampleBase64}`);
    });

    it("sets correct data URL for GIF", () => {
      render(<ImagePreview content={sampleBase64} extension="gif" fileName="test.gif" />);

      const img = screen.getByRole("img");
      expect(img).toHaveAttribute("src", `data:image/gif;base64,${sampleBase64}`);
    });

    it("sets correct data URL for WebP", () => {
      render(<ImagePreview content={sampleBase64} extension="webp" fileName="test.webp" />);

      const img = screen.getByRole("img");
      expect(img).toHaveAttribute("src", `data:image/webp;base64,${sampleBase64}`);
    });
  });

  describe("case insensitivity", () => {
    it("handles uppercase extension", () => {
      render(<ImagePreview content={sampleBase64} extension="PNG" fileName="test.PNG" />);

      const img = screen.getByRole("img");
      expect(img).toHaveAttribute("src", `data:image/png;base64,${sampleBase64}`);
    });
  });
});

// ============================================
// parseFrontmatter Tests
// ============================================

describe("parseFrontmatter", () => {
  it("returns no entries for content without frontmatter", () => {
    const content = "# Hello\n\nSome content";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([]);
    expect(result.body).toBe(content);
  });

  it("parses simple key-value pairs", () => {
    const content = "---\ntitle: My Page\ndate: 2025-01-15\n---\n\n# Hello";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([
      { key: "title", value: "My Page" },
      { key: "date", value: "2025-01-15" },
    ]);
    expect(result.body).toBe("# Hello");
  });

  it("handles array values in bracket notation", () => {
    const content = "---\ntags: [react, typescript]\n---\n\nBody";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([{ key: "tags", value: "[react, typescript]" }]);
    expect(result.body).toBe("Body");
  });

  it("handles empty frontmatter block", () => {
    const content = "---\n---\n\n# Content";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([]);
    expect(result.body).toBe("# Content");
  });

  it("handles boolean values", () => {
    const content = "---\ndraft: false\npublished: true\n---\nBody";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([
      { key: "draft", value: "false" },
      { key: "published", value: "true" },
    ]);
  });

  it("returns original content when no closing delimiter found", () => {
    const content = "---\ntitle: Broken\n\n# Content";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([]);
    expect(result.body).toBe(content);
  });

  it("does not parse frontmatter that doesn't start at beginning", () => {
    const content = "\n---\ntitle: Not frontmatter\n---\nBody";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([]);
    expect(result.body).toBe(content);
  });

  it("handles values with colons", () => {
    const content = "---\nurl: https://example.com\n---\nBody";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([{ key: "url", value: "https://example.com" }]);
  });

  it("handles empty values", () => {
    const content = "---\ntitle:\n---\nBody";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([{ key: "title", value: "" }]);
  });

  it("handles multiline values with indentation", () => {
    const content = "---\ndescription: First line\n  continued here\ntitle: Test\n---\nBody";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([
      { key: "description", value: "First line\ncontinued here" },
      { key: "title", value: "Test" },
    ]);
  });

  it("handles keys with hyphens and underscores", () => {
    const content = "---\nfull-title: Hello\nmy_key: World\n---\nBody";
    const result = parseFrontmatter(content);
    expect(result.entries).toEqual([
      { key: "full-title", value: "Hello" },
      { key: "my_key", value: "World" },
    ]);
  });
});

// ============================================
// MarkdownPreview Tests
// ============================================

describe("MarkdownPreview", () => {
  beforeEach(() => {
    capturedStreamdownProps = {};
  });

  it("renders markdown content without frontmatter", () => {
    render(<MarkdownPreview content="# Hello World" />);
    expect(screen.getByTestId("streamdown-mock")).toHaveTextContent("# Hello World");
  });

  it("passes the icon link renderer to readme markdown links", () => {
    render(<MarkdownPreview content="[PR](https://github.com/everruns/everruns/pull/44)" />);

    const components = capturedStreamdownProps.components as Record<string, unknown>;
    expect(components.a).toBe(MarkdownLink);
  });

  it("strips frontmatter and renders body", () => {
    const content = "---\ntitle: Test\n---\n\n# Hello";
    render(<MarkdownPreview content={content} />);
    expect(screen.getByTestId("streamdown-mock")).toHaveTextContent("# Hello");
  });

  it("displays frontmatter entries as metadata", () => {
    const content = "---\ntitle: My Page\nauthor: Jane\n---\n\n# Content";
    render(<MarkdownPreview content={content} />);
    expect(screen.getByText("title")).toBeInTheDocument();
    expect(screen.getByText("My Page")).toBeInTheDocument();
    expect(screen.getByText("author")).toBeInTheDocument();
    expect(screen.getByText("Jane")).toBeInTheDocument();
  });

  it("does not render frontmatter block when none present", () => {
    const content = "# Just Markdown";
    const { container } = render(<MarkdownPreview content={content} />);
    expect(container.querySelector(".file-preview-frontmatter")).not.toBeInTheDocument();
  });
});

// ============================================
// SVGPreview Tests (sandboxed iframe path)
// ============================================
//
// These tests verify two things:
// 1. The trust-gate wiring: a sandboxed iframe is rendered with `sandbox=""`
//    and a strict `Content-Security-Policy` meta tag inside the srcDoc.
// 2. The legitimate SVG content survives intact, but XSS payloads
//    (`<script>`, `on*` handlers, `javascript:` URLs, `<foreignObject>`
//    HTML) are present in the sandboxed document body — they are NOT
//    stripped server-side. The iframe sandbox + CSP do the gating, not
//    text-level sanitization. The tests assert the SVG payload reaches
//    the iframe so the gate is actually exercised.

describe("SVGPreview", () => {
  function getIframeSrcDoc(container: HTMLElement): string {
    const iframe = container.querySelector("iframe");
    expect(iframe).toBeInTheDocument();
    return iframe?.getAttribute("srcdoc") ?? "";
  }

  describe("sandbox + CSP", () => {
    it("renders an iframe with empty sandbox attribute", () => {
      const svg = "<svg xmlns='http://www.w3.org/2000/svg'><circle r='10'/></svg>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      const iframe = container.querySelector("iframe");
      expect(iframe).toBeInTheDocument();
      // empty sandbox = deny all flags (scripts, forms, popups, top-nav, same-origin)
      expect(iframe?.getAttribute("sandbox")).toBe("");
    });

    it("includes strict CSP in iframe srcdoc", () => {
      const svg = "<svg xmlns='http://www.w3.org/2000/svg'/>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      const srcDoc = getIframeSrcDoc(container);
      expect(srcDoc).toContain("Content-Security-Policy");
      expect(srcDoc).toContain("default-src 'none'");
      expect(srcDoc).toContain("style-src 'unsafe-inline'");
      // remote img loads must stay blocked; only data: is allowed for inline raster
      expect(srcDoc).toMatch(/img-src\s+data:/);
    });

    it("renders SVG body inside the sandboxed iframe", () => {
      const svg = "<svg xmlns='http://www.w3.org/2000/svg'><rect width='10' height='10'/></svg>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      const srcDoc = getIframeSrcDoc(container);
      expect(srcDoc).toContain("<rect width='10' height='10'/>");
    });
  });

  describe("XSS payloads remain inside sandbox", () => {
    // We do NOT sanitize the SVG body; we rely on the sandbox + CSP. These
    // tests assert that the dangerous markup does reach the iframe srcdoc
    // (proving the sandbox is exercising it) AND that nothing in the host
    // document was rendered or executed.

    it("does not execute <script> outside the iframe", () => {
      const svg =
        "<svg xmlns='http://www.w3.org/2000/svg'><script>window.__svg_pwned=true</script></svg>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      // payload is inside the sandboxed srcdoc, not in host DOM
      expect(container.querySelector("script")).not.toBeInTheDocument();
      expect((window as unknown as { __svg_pwned?: boolean }).__svg_pwned).toBeUndefined();
      const srcDoc = getIframeSrcDoc(container);
      expect(srcDoc).toContain("<script>");
    });

    it("keeps on* event-handler attributes inside the sandbox", () => {
      const svg =
        "<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'><circle r='5' onclick='alert(2)'/></svg>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      // host DOM has no <svg> elements with handlers
      expect(container.querySelector("svg")).not.toBeInTheDocument();
      const srcDoc = getIframeSrcDoc(container);
      // payload reached the sandbox; CSP + sandbox stop execution
      expect(srcDoc).toContain("onload='alert(1)'");
      expect(srcDoc).toContain("onclick='alert(2)'");
    });

    it("keeps javascript: URLs inside the sandbox", () => {
      const svg =
        "<svg xmlns='http://www.w3.org/2000/svg'><a xlink:href='javascript:alert(1)'><circle r='5'/></a></svg>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      // host DOM has no anchor with the javascript: URL
      expect(container.querySelector("a[href^='javascript:']")).not.toBeInTheDocument();
      const srcDoc = getIframeSrcDoc(container);
      expect(srcDoc).toContain("javascript:alert(1)");
    });

    it("keeps <foreignObject> HTML inside the sandbox", () => {
      const svg =
        "<svg xmlns='http://www.w3.org/2000/svg'><foreignObject width='100' height='100'><div xmlns='http://www.w3.org/1999/xhtml'><img src=x onerror='alert(1)'/></div></foreignObject></svg>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      // no foreignObject HTML escaped into the host DOM
      expect(container.querySelector("foreignObject")).not.toBeInTheDocument();
      expect(container.querySelector("img[onerror]")).not.toBeInTheDocument();
      const srcDoc = getIframeSrcDoc(container);
      expect(srcDoc).toContain("foreignObject");
    });
  });

  describe("input variants", () => {
    it("decodes base64-encoded SVG into the iframe", () => {
      const svg = "<svg xmlns='http://www.w3.org/2000/svg'><rect width='5' height='5'/></svg>";
      const base64 = Buffer.from(svg, "utf8").toString("base64");
      const { container } = render(<SVGPreview content={base64} encoding="base64" />);
      const srcDoc = getIframeSrcDoc(container);
      expect(srcDoc).toContain("<rect width='5' height='5'/>");
    });

    it("decodes base64 SVG with embedded whitespace/newlines", () => {
      // PEM-style line wrapping every 64 chars or pasted-from-clipboard
      // payloads commonly include whitespace; strict `atob` rejects them.
      const svg = "<svg xmlns='http://www.w3.org/2000/svg'><rect width='5' height='5'/></svg>";
      const base64 = Buffer.from(svg, "utf8").toString("base64");
      const wrapped = base64.replace(/(.{16})/g, "$1\n");
      const { container } = render(<SVGPreview content={wrapped} encoding="base64" />);
      const srcDoc = getIframeSrcDoc(container);
      expect(srcDoc).toContain("<rect width='5' height='5'/>");
    });

    it("renders SVGs that happen to contain the substring </body>", () => {
      // Defensive regression: a comment containing `</body>` must still
      // render through the sandbox path rather than being blanked out.
      const svg =
        "<svg xmlns='http://www.w3.org/2000/svg'><!-- the string </body> appears here --><rect width='1' height='1'/></svg>";
      const { container } = render(<SVGPreview content={svg} encoding="text" />);
      const iframe = container.querySelector("iframe");
      expect(iframe).toBeInTheDocument();
      const srcDoc = iframe?.getAttribute("srcdoc") ?? "";
      expect(srcDoc).toContain("<rect width='1' height='1'/>");
    });

    it("shows empty-state for whitespace-only SVG", () => {
      const { container } = render(<SVGPreview content="   " encoding="text" />);
      expect(container.querySelector("iframe")).not.toBeInTheDocument();
      expect(screen.getByText("Empty or invalid SVG")).toBeInTheDocument();
    });

    it("shows empty-state for unparseable base64", () => {
      const { container } = render(<SVGPreview content="!!!not-base64!!!" encoding="base64" />);
      expect(container.querySelector("iframe")).not.toBeInTheDocument();
      expect(screen.getByText("Empty or invalid SVG")).toBeInTheDocument();
    });
  });
});

// ============================================
// HtmlPreview Tests (sandboxed, opaque-origin iframe path)
// ============================================
//
// Two modes (see HtmlPreview): server-backed (`src` → JS runs via the endpoint's
// own `sandbox allow-scripts` response CSP) and static fallback (`srcDoc` →
// isolated, no JS under the app CSP). Both run in an opaque origin: `sandbox`
// has `allow-scripts` but never `allow-same-origin`/top-nav/forms/popups.

describe("HtmlPreview", () => {
  function getIframe(container: HTMLElement): HTMLIFrameElement {
    const iframe = container.querySelector("iframe");
    expect(iframe).toBeInTheDocument();
    return iframe as HTMLIFrameElement;
  }

  describe("server-backed mode (src)", () => {
    it("loads the preview endpoint via src with an opaque-origin sandbox", () => {
      const url = "/api/v1/workspaces/wsp_x/fs/_/preview/index.html";
      const { container } = render(<HtmlPreview content="<p>ignored</p>" src={url} />);
      const iframe = getIframe(container);
      expect(iframe.getAttribute("src")).toBe(url);
      // srcDoc must not be used in server-backed mode.
      expect(iframe.getAttribute("srcdoc")).toBeNull();
      expect(iframe.getAttribute("sandbox")).toBe("allow-scripts");
      expect(iframe.getAttribute("sandbox")).not.toContain("allow-same-origin");
      expect(iframe.getAttribute("referrerpolicy")).toBe("no-referrer");
    });
  });

  it("renders an iframe that allows scripts but not same-origin", () => {
    const { container } = render(<HtmlPreview content="<p>hi</p>" />);
    const iframe = getIframe(container);
    // allow-scripts WITHOUT allow-same-origin keeps the document opaque-origin.
    expect(iframe.getAttribute("sandbox")).toBe("allow-scripts");
    expect(iframe.getAttribute("sandbox")).not.toContain("allow-same-origin");
    expect(iframe.getAttribute("referrerpolicy")).toBe("no-referrer");
  });

  it("does not grant top-navigation, forms, popups, or modals", () => {
    const { container } = render(<HtmlPreview content="<p>hi</p>" />);
    const sandbox = getIframe(container).getAttribute("sandbox") ?? "";
    expect(sandbox).not.toContain("allow-top-navigation");
    expect(sandbox).not.toContain("allow-forms");
    expect(sandbox).not.toContain("allow-popups");
    expect(sandbox).not.toContain("allow-modals");
  });

  it("passes the HTML (including scripts) through to the srcdoc verbatim", () => {
    const html =
      "<html><head><title>T</title></head><body><script>window.x=1</script>hi</body></html>";
    const { container } = render(<HtmlPreview content={html} />);
    const srcDoc = getIframe(container).getAttribute("srcdoc") ?? "";
    // No host-document script element; the payload lives only in the sandbox.
    expect(container.querySelector("script")).not.toBeInTheDocument();
    expect(srcDoc).toContain("<script>window.x=1</script>");
    expect(srcDoc).toContain("hi");
  });

  it("injects a hardening CSP meta as the first child of <head>", () => {
    const html = "<html><head><title>T</title></head><body>hi</body></html>";
    const { container } = render(<HtmlPreview content={html} />);
    const srcDoc = getIframe(container).getAttribute("srcdoc") ?? "";
    expect(srcDoc).toContain('http-equiv="Content-Security-Policy"');
    expect(srcDoc).toContain("object-src 'none'");
    expect(srcDoc).toContain("base-uri 'none'");
    expect(srcDoc).toContain("form-action 'none'");
    // The meta must precede the user's head content so it governs it.
    expect(srcDoc.indexOf("Content-Security-Policy")).toBeLessThan(srcDoc.indexOf("<title>"));
    // The CSP intentionally does NOT constrain scripts (preview must run JS).
    expect(srcDoc).not.toContain("script-src");
  });

  it("synthesizes a head when the HTML has none", () => {
    const { container } = render(<HtmlPreview content="<html><body>hi</body></html>" />);
    const srcDoc = getIframe(container).getAttribute("srcdoc") ?? "";
    expect(srcDoc).toContain("<head>");
    expect(srcDoc).toContain("Content-Security-Policy");
    expect(srcDoc.indexOf("Content-Security-Policy")).toBeLessThan(srcDoc.indexOf("<body>"));
  });

  it("prepends the CSP for bare HTML fragments", () => {
    const { container } = render(<HtmlPreview content="<p>just a fragment</p>" />);
    const srcDoc = getIframe(container).getAttribute("srcdoc") ?? "";
    expect(srcDoc.startsWith("<meta")).toBe(true);
    expect(srcDoc).toContain("<p>just a fragment</p>");
  });
});

// ============================================
// PdfPreview Tests (data: URL iframe path)
// ============================================
//
// Chromium disables its PDF viewer inside any sandboxed iframe, so the security
// boundary is the data: URL's opaque origin plus the out-of-process viewer.
// These tests assert the data: URL wiring and the forced application/pdf type.

describe("PdfPreview", () => {
  const sampleBase64 = "JVBERi0xLjQKJUVPRg==";

  it("renders an iframe pointing at a data:application/pdf URL", () => {
    const { container } = render(<PdfPreview content={sampleBase64} />);
    const iframe = container.querySelector("iframe");
    expect(iframe).toBeInTheDocument();
    expect(iframe?.getAttribute("src")).toBe(`data:application/pdf;base64,${sampleBase64}`);
  });

  it("strips whitespace/newlines from base64 before building the data URL", () => {
    const wrapped = sampleBase64.replace(/(.{4})/g, "$1\n");
    const { container } = render(<PdfPreview content={wrapped} />);
    const iframe = container.querySelector("iframe");
    expect(iframe?.getAttribute("src")).toBe(`data:application/pdf;base64,${sampleBase64}`);
  });

  it("shows an empty-state for blank content", () => {
    const { container } = render(<PdfPreview content="   " />);
    expect(container.querySelector("iframe")).not.toBeInTheDocument();
    expect(screen.getByText("Empty or invalid PDF")).toBeInTheDocument();
  });
});
