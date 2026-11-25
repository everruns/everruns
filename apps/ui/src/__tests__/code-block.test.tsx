import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const highlight = jest.fn((code: string, options: { lang: string }) => ({
  tokens: code.split("\n").map((line) => [
    {
      content: line,
      offset: 0,
      htmlStyle: { color: `highlight-${options.lang}`, "--shiki-dark": "dark-highlight" },
    },
  ]),
}));

jest.mock("@pierre/highlights", () => ({
  codeToTokens: (code: string, options: { lang: string }) => highlight(code, options),
  isSupportedLanguage: () => true,
}));

import { CodeBlock } from "@/components/ui/code-block";

const samples = [
  { label: "Python", language: "python", code: "def main():\n    return True" },
  { label: "TypeScript", language: "ts", code: "const ready = true;" },
  { label: "Rust", language: "rust", code: "fn main() {}" },
];

describe("CodeBlock", () => {
  beforeEach(() => {
    highlight.mockClear();
  });

  it("highlights Python, TypeScript, and Rust when their tabs are selected", async () => {
    render(<CodeBlock samples={samples} />);

    await waitFor(() =>
      expect(highlight).toHaveBeenCalledWith(
        expect.any(String),
        expect.objectContaining({ lang: "python" }),
      ),
    );

    fireEvent.click(screen.getByRole("button", { name: "TypeScript" }));
    await waitFor(() =>
      expect(highlight).toHaveBeenCalledWith(
        expect.any(String),
        expect.objectContaining({ lang: "ts" }),
      ),
    );

    fireEvent.click(screen.getByRole("button", { name: "Rust" }));
    await waitFor(() =>
      expect(highlight).toHaveBeenCalledWith(
        expect.any(String),
        expect.objectContaining({ lang: "rust" }),
      ),
    );

    expect(screen.getByText("fn main() {}")).toHaveStyle({
      "--code-token-light": "highlight-rust",
    });
  });

  it("copies the raw code from the active tab", async () => {
    const writeText = jest.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(<CodeBlock samples={samples} />);
    await waitFor(() => expect(highlight).toHaveBeenCalled());

    fireEvent.click(screen.getByRole("button", { name: "TypeScript" }));
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "Copy code" }));
    });

    expect(writeText).toHaveBeenCalledWith("const ready = true;");
    expect(screen.getByRole("button", { name: "Copied" })).toBeInTheDocument();
  });
});
