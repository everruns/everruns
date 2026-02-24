import { render, screen } from "@testing-library/react";

// Mock streamdown to avoid ESM issues in Jest
jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => (
    <pre data-testid="streamdown-mock">{children}</pre>
  ),
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
  parseFrontmatter,
} from "@/components/files/file-previews";

// --------------------------------------------
// getPreviewType Tests
// --------------------------------------------

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
      expect(getPreviewType("html", "text")).toBe("code");
      expect(getPreviewType("css", "text")).toBe("code");
      expect(getPreviewType("scss", "text")).toBe("code");
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

    it("returns 'image' for SVG files with base64 encoding", () => {
      expect(getPreviewType("svg", "base64")).toBe("image");
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

// --------------------------------------------
// canPreview Tests
// --------------------------------------------

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
  });

  describe("non-previewable files", () => {
    it("returns false for plain text files", () => {
      expect(canPreview("txt", "text")).toBe(false);
    });

    it("returns false for binary files", () => {
      expect(canPreview("exe", "base64")).toBe(false);
      expect(canPreview("bin", "base64")).toBe(false);
    });

    it("returns false for unknown file types", () => {
      expect(canPreview("xyz", "text")).toBe(false);
    });
  });
});

// --------------------------------------------
// CSVPreview Tests
// --------------------------------------------

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

// --------------------------------------------
// ImagePreview Tests
// --------------------------------------------

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

    it("sets correct data URL for SVG", () => {
      render(<ImagePreview content={sampleBase64} extension="svg" fileName="test.svg" />);

      const img = screen.getByRole("img");
      expect(img).toHaveAttribute("src", `data:image/svg+xml;base64,${sampleBase64}`);
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

// --------------------------------------------
// parseFrontmatter Tests
// --------------------------------------------

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

// --------------------------------------------
// MarkdownPreview Tests
// --------------------------------------------

describe("MarkdownPreview", () => {
  it("renders markdown content without frontmatter", () => {
    render(<MarkdownPreview content="# Hello World" />);
    expect(screen.getByTestId("streamdown-mock")).toHaveTextContent("# Hello World");
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
