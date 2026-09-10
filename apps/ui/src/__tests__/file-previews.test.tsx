import { MarkdownLink } from "@/components/markdown/markdown-link";
import { act, render, screen } from "@testing-library/react";

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

describe("preview classification", () => {
  it("classifies supported text extensions and gates the preview consistently", () => {
    const groups = [
      [
        [
          "ts",
          "tsx",
          "js",
          "jsx",
          "py",
          "rs",
          "go",
          "sh",
          "bash",
          "zsh",
          "yml",
          "yaml",
          "toml",
          "css",
          "scss",
          "java",
          "c",
          "cpp",
          "h",
          "hpp",
          "rb",
          "php",
          "sql",
          "xml",
          "sass",
          "less",
          "vue",
          "svelte",
        ],
        "code",
        true,
      ],
      [["html", "htm"], "html", true],
      [["md", "markdown"], "markdown", true],
      [["json"], "json", true],
      [["csv"], "csv", true],
      [["svg"], "svg", true],
      [["txt", "xyz", "unknown", "", "png", "jpg", "pdf"], "text", false],
    ] as const;
    for (const [extensions, type, previewable] of groups) {
      for (const extension of extensions) {
        for (const variant of [
          extension,
          extension.toUpperCase(),
          extension.slice(0, 1).toUpperCase() + extension.slice(1),
        ]) {
          expect({
            extension: variant,
            type: getPreviewType(variant, "text"),
            previewable: canPreview(variant, "text"),
          }).toEqual({ extension: variant, type, previewable });
        }
      }
    }
  });

  it("uses encoded binary routing even for extensions normally rendered as text", () => {
    const cases = [
      ["png", "image", true],
      ["jpg", "image", true],
      ["jpeg", "image", true],
      ["gif", "image", true],
      ["webp", "image", true],
      ["bmp", "image", true],
      ["ico", "image", true],
      ["pdf", "pdf", true],
      ["svg", "svg", true],
      ["exe", "binary", false],
      ["bin", "binary", false],
      ["dll", "binary", false],
      ["ts", "binary", false],
      ["html", "binary", false],
      ["md", "binary", false],
      ["json", "binary", false],
      ["csv", "binary", false],
      ["", "binary", false],
    ] as const;
    for (const [extension, type, previewable] of cases) {
      for (const variant of [extension, extension.toUpperCase()]) {
        expect({
          extension: variant,
          type: getPreviewType(variant, "base64"),
          previewable: canPreview(variant, "base64"),
        }).toEqual({ extension: variant, type, previewable });
      }
    }
  });
});

// ============================================
// CSVPreview Tests
// ============================================

describe("CSVPreview", () => {
  it("renders complete ordered records, quoted fields, and accurate dimensions", async () => {
    const { container, rerender } = render(<CSVPreview content="" />);
    const cases = [
      [
        "name,age,city\nAlice,30,NYC\nBob,25,LA",
        [
          ["name", "age", "city"],
          ["Alice", "30", "NYC"],
          ["Bob", "25", "LA"],
        ],
        "2 rows, 3 columns",
      ],
      [
        'name,address,quote\nJohn,"123 Main St, Apt 4","She said ""hello"""',
        [
          ["name", "address", "quote"],
          ["John", "123 Main St, Apt 4", 'She said "hello"'],
        ],
        "1 rows, 3 columns",
      ],
      [
        "names\nAlice\nBob\nCharlie",
        [["names"], ["Alice"], ["Bob"], ["Charlie"]],
        "3 rows, 1 columns",
      ],
      [
        'name,note\nAlice,"first\nsecond"\nBob,done',
        [
          ["name", "note"],
          ["Alice", "first\nsecond"],
          ["Bob", "done"],
        ],
        "2 rows, 2 columns",
      ],
      [
        'name,note\r\nAlice,"first\r\nsecond"\r\nBob,done',
        [
          ["name", "note"],
          ["Alice", "first\r\nsecond"],
          ["Bob", "done"],
        ],
        "2 rows, 2 columns",
      ],
      [
        "a,b\n\n1,\n  \n,2\n",
        [
          ["a", "b"],
          ["1", ""],
          ["", "2"],
        ],
        "2 rows, 2 columns",
      ],
      ["header", [["header"]], "0 rows, 1 columns"],
    ] as const;
    for (const [content, cells, dimensions] of cases) {
      await act(async () => {
        rerender(<CSVPreview content={content} />);
      });
      expect(
        Array.from(container.querySelectorAll("tr"), (row) =>
          Array.from(row.querySelectorAll("th,td"), (cell) => cell.textContent),
        ),
      ).toEqual(cells);
      expect(screen.getByText(dimensions)).toBeInTheDocument();
    }
  });

  it("removes the table when the input becomes empty", async () => {
    const { container, rerender } = render(<CSVPreview content="name\nAlice" />);
    for (const content of ["", " \t\r\n\n"]) {
      await act(async () => {
        rerender(<CSVPreview content={content} />);
      });
      expect(container.querySelector("table")).toBeNull();
      expect(screen.getByText("Empty or invalid CSV")).toBeInTheDocument();
    }
  });
});

// ============================================
// ImagePreview Tests
// ============================================

describe("ImagePreview", () => {
  it("preserves the bytes and accessible filename while selecting the image MIME type", async () => {
    const content =
      "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";
    const { rerender } = render(
      <ImagePreview content={content} extension="png" fileName="first.png" />,
    );
    for (const [extension, mime] of [
      ["png", "image/png"],
      ["jpg", "image/jpeg"],
      ["jpeg", "image/jpeg"],
      ["gif", "image/gif"],
      ["webp", "image/webp"],
      ["bmp", "image/bmp"],
      ["ico", "image/x-icon"],
      ["unknown", "image/png"],
    ]) {
      for (const variant of [extension, extension.toUpperCase()]) {
        const fileName = `diagram.${variant}`;
        await act(async () => {
          rerender(<ImagePreview content={content} extension={variant} fileName={fileName} />);
        });
        expect(screen.getByRole("img", { name: fileName })).toHaveAttribute(
          "src",
          `data:${mime};base64,${content}`,
        );
      }
    }
  });
});

// ============================================
// parseFrontmatter Tests
// ============================================

describe("parseFrontmatter", () => {
  it("preserves the complete input unless both delimiter lines are valid", () => {
    for (const content of [
      "# Hello\n\nSome content",
      "\n---\ntitle: Not frontmatter\n---\nBody",
      "---\ntitle: Unclosed\n\n# Content",
      "---not-frontmatter\ntitle: Keep\n---\nBody",
      "----\ntitle: Keep\n---\nBody",
      "---\ntitle: Keep\n---suffix\nBody",
      "---\ntitle: Keep\n----\nBody",
      "---",
      "",
    ]) {
      expect({ content, result: parseFrontmatter(content) }).toEqual({
        content,
        result: { entries: [], body: content },
      });
    }
  });

  it("returns every metadata value and the exact remaining body", () => {
    const metadata = [
      "title: My Page",
      "date: 2025-01-15",
      "tags: [react, typescript]",
      "draft: false",
      "published: true",
      "url: https://example.com/a:b",
      "empty:",
      "description: First line",
      "  continued here",
      "\tlast line",
      "full-title: Hello",
      "my_key: World",
    ];
    for (const newline of ["\n", "\r\n"]) {
      const content = ["---", ...metadata, "---", "", "# Body", "", "Trailing  "].join(newline);
      expect(parseFrontmatter(content)).toEqual({
        entries: [
          { key: "title", value: "My Page" },
          { key: "date", value: "2025-01-15" },
          { key: "tags", value: "[react, typescript]" },
          { key: "draft", value: "false" },
          { key: "published", value: "true" },
          { key: "url", value: "https://example.com/a:b" },
          { key: "empty", value: "" },
          { key: "description", value: "First line\ncontinued here\nlast line" },
          { key: "full-title", value: "Hello" },
          { key: "my_key", value: "World" },
        ],
        body: ["# Body", "", "Trailing  "].join(newline),
      });
    }
  });

  it("handles empty metadata and a closing delimiter at end of input", () => {
    for (const [content, expected] of [
      ["---\n---\n\n# Content", { entries: [], body: "# Content" }],
      ["---\r\n---\r\nBody", { entries: [], body: "Body" }],
      ["---\n---", { entries: [], body: "" }],
      ["---\ntitle: End\n---", { entries: [{ key: "title", value: "End" }], body: "" }],
    ] as const) {
      expect(parseFrontmatter(content)).toEqual(expected);
    }
  });
});

describe("MarkdownPreview", () => {
  it("passes only the body to Streamdown and renders complete metadata separately", async () => {
    const body = "# Hello\n\n[PR](https://github.com/everruns/everruns/pull/44)\n";
    const { container, rerender } = render(
      <MarkdownPreview content={`---\ntitle: My Page\nauthor: Jane\nempty:\n---\n\n${body}`} />,
    );
    expect(capturedStreamdownProps.children).toBe(body);
    expect((capturedStreamdownProps.components as Record<string, unknown>).a).toBe(MarkdownLink);
    expect(
      Array.from(container.querySelectorAll("tr"), (row) =>
        Array.from(row.querySelectorAll("td"), (cell) => cell.textContent),
      ),
    ).toEqual([
      ["title", "My Page"],
      ["author", "Jane"],
      ["empty", "—"],
    ]);

    for (const content of [
      body,
      "---invalid\ntitle: Keep\n---\n# Body",
      "---\n---\n# Empty metadata",
    ]) {
      await act(async () => {
        rerender(<MarkdownPreview content={content} />);
      });
      expect(capturedStreamdownProps.children).toBe(
        content.startsWith("---\n---") ? "# Empty metadata" : content,
      );
      expect(container.querySelector("table")).toBeNull();
    }
  });
});

// These unit tests verify iframe attributes and payload isolation. JSDOM does
// not execute iframe documents; browser enforcement needs browser tests.
describe("SVGPreview", () => {
  it("isolates intact SVG payloads behind the exact sandbox and CSP", async () => {
    const { container, rerender } = render(<SVGPreview content="" encoding="text" />);
    for (const svg of [
      "<svg xmlns='http://www.w3.org/2000/svg'><rect width='10' height='10'/></svg>",
      "<svg xmlns='http://www.w3.org/2000/svg'><script>window.__svg_pwned=true</script></svg>",
      "<svg xmlns='http://www.w3.org/2000/svg' onload='alert(1)'><circle r='5' onclick='alert(2)'/></svg>",
      "<svg xmlns='http://www.w3.org/2000/svg'><a xlink:href='javascript:alert(1)'><circle r='5'/></a></svg>",
      "<svg xmlns='http://www.w3.org/2000/svg'><foreignObject width='100' height='100'><div xmlns='http://www.w3.org/1999/xhtml'><img src=x onerror='alert(1)'/></div></foreignObject></svg>",
      "<svg xmlns='http://www.w3.org/2000/svg'><!-- the string </body> appears here --><rect width='1' height='1'/></svg>",
    ]) {
      await act(async () => {
        rerender(<SVGPreview content={svg} encoding="text" />);
      });
      const iframe = screen.getByTitle("SVG preview");
      expect(iframe).toHaveAttribute("sandbox", "");
      expect(iframe).not.toHaveAttribute("src");
      const srcDoc = iframe.getAttribute("srcdoc")!;
      expect(srcDoc).toContain(`<body>${svg}</body>`);
      const document = new DOMParser().parseFromString(srcDoc, "text/html");
      expect(
        Array.from(
          document.querySelectorAll('meta[http-equiv="Content-Security-Policy"]'),
          (meta) => meta.getAttribute("content"),
        ),
      ).toEqual(["default-src 'none'; style-src 'unsafe-inline'; img-src data:"]);
      expect(
        container.querySelector("svg, script, foreignObject, img[onerror], a[href^='javascript:']"),
      ).toBeNull();
    }
  });

  it("decodes plain and whitespace-wrapped base64 without changing the SVG", async () => {
    const svg = "<svg xmlns='http://www.w3.org/2000/svg'><rect width='5' height='5'/></svg>";
    const base64 = Buffer.from(svg, "utf8").toString("base64");
    const { rerender } = render(<SVGPreview content={base64} encoding="base64" />);
    for (const content of [base64, ` \t${base64.replace(/(.{16})/g, "$1\r\n")} `]) {
      await act(async () => {
        rerender(<SVGPreview content={content} encoding="base64" />);
      });
      expect(screen.getByTitle("SVG preview").getAttribute("srcdoc")).toContain(
        `<body>${svg}</body>`,
      );
    }
  });

  it("replaces the iframe with an empty state for blank or invalid encoded input", async () => {
    const { container, rerender } = render(<SVGPreview content="<svg/>" encoding="text" />);
    for (const [content, encoding] of [
      ["   ", "text"],
      ["", "text"],
      ["!!!not-base64!!!", "base64"],
      [" \n", "base64"],
    ] as const) {
      await act(async () => {
        rerender(<SVGPreview content={content} encoding={encoding} />);
      });
      expect(container.querySelector("iframe")).toBeNull();
      expect(screen.getByText("Empty or invalid SVG")).toBeInTheDocument();
    }
  });
});

describe("HtmlPreview", () => {
  it("loads server previews with the exact sandbox and without srcdoc", () => {
    const src = "/api/v1/workspaces/wsp_x/fs/_/preview/index.html";
    render(<HtmlPreview content="<p>ignored</p>" src={src} />);
    const iframe = screen.getByTitle("HTML preview");
    expect(iframe).toHaveAttribute("src", src);
    expect(iframe).not.toHaveAttribute("srcdoc");
    expect(iframe).toHaveAttribute("sandbox", "allow-scripts");
    expect(iframe).toHaveAttribute("referrerpolicy", "no-referrer");
  });

  it("places the exact hardening policy before user content and preserves every input byte", async () => {
    const meta =
      "<meta http-equiv=\"Content-Security-Policy\" content=\"object-src 'none'; base-uri 'none'; form-action 'none'\">";
    const cases = [
      [
        "<html><head><title>T</title></head><body><script>window.x=1</script>hi</body></html>",
        `<html><head>${meta}<title>T</title></head><body><script>window.x=1</script>hi</body></html>`,
      ],
      [
        "<HTML lang='en'><HEAD data-theme='a'><title>T</title></HEAD><body>hi</body></HTML>",
        `<HTML lang='en'><HEAD data-theme='a'>${meta}<title>T</title></HEAD><body>hi</body></HTML>`,
      ],
      ["<html><body>hi</body></html>", `<html><head>${meta}</head><body>hi</body></html>`],
      ["<p>just a fragment</p>", `${meta}<p>just a fragment</p>`],
      ["<header><h1>Heading</h1></header>", `${meta}<header><h1>Heading</h1></header>`],
      ["<html-widget>custom</html-widget>", `${meta}<html-widget>custom</html-widget>`],
    ];
    const { container, rerender } = render(<HtmlPreview content="" />);
    for (const [content, expected] of cases) {
      await act(async () => {
        rerender(<HtmlPreview content={content} />);
      });
      const iframe = screen.getByTitle("HTML preview");
      expect(iframe).toHaveAttribute("sandbox", "allow-scripts");
      expect(iframe).toHaveAttribute("referrerpolicy", "no-referrer");
      expect(iframe).not.toHaveAttribute("src");
      expect(iframe.getAttribute("srcdoc")).toBe(expected);
      const document = new DOMParser().parseFromString(expected, "text/html");
      expect(document.head.firstElementChild?.getAttribute("content")).toBe(
        "object-src 'none'; base-uri 'none'; form-action 'none'",
      );
      expect(container.querySelector("script, header, h1")).toBeNull();
    }
  });
});

describe("PdfPreview", () => {
  it("forces PDF MIME and preserves normalized bytes without sandboxing the browser viewer", async () => {
    const base64 = "JVBERi0xLjQKJUVPRg==";
    const { rerender } = render(<PdfPreview content={base64} />);
    for (const content of [base64, ` \t${base64.replace(/(.{4})/g, "$1\r\n")} `]) {
      await act(async () => {
        rerender(<PdfPreview content={content} />);
      });
      const iframe = screen.getByTitle("PDF preview");
      expect(iframe).toHaveAttribute("src", `data:application/pdf;base64,${base64}`);
      expect(iframe).not.toHaveAttribute("sandbox");
      expect(iframe).not.toHaveAttribute("srcdoc");
    }
  });

  it("removes the viewer when its content becomes blank", async () => {
    const { container, rerender } = render(<PdfPreview content="JVBERi0xLjQKJUVPRg==" />);
    for (const content of ["", " \t\n"]) {
      await act(async () => {
        rerender(<PdfPreview content={content} />);
      });
      expect(container.querySelector("iframe")).toBeNull();
      expect(screen.getByText("Empty or invalid PDF")).toBeInTheDocument();
    }
  });
});
