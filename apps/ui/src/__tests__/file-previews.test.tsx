import { render, screen } from "@testing-library/react";

// Mock streamdown to avoid ESM issues in Jest
jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => <pre data-testid="streamdown-mock">{children}</pre>,
}));

jest.mock("@streamdown/code", () => ({
  code: {},
}));

import {
  getPreviewType,
  canPreview,
  CSVPreview,
  ImagePreview,
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
