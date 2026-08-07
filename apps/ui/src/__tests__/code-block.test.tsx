import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";

const highlight = jest.fn(
  (
    options: { code: string; language: string },
    callback?: (result: {
      tokens: Array<Array<{ content: string; offset: number; htmlStyle: object }>>;
    }) => void,
  ) => {
    const result = {
      tokens: options.code.split("\n").map((line) => [
        {
          content: line,
          offset: 0,
          htmlStyle: { color: `highlight-${options.language}`, "--shiki-dark": "dark-highlight" },
        },
      ]),
    };
    callback?.(result);
    return null;
  },
);

jest.mock("@streamdown/code", () => ({
  code: {
    getThemes: () => ["github-light", "github-dark"],
    highlight,
    supportsLanguage: () => true,
  },
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
        expect.objectContaining({ language: "python" }),
        expect.any(Function),
      ),
    );

    fireEvent.click(screen.getByRole("button", { name: "TypeScript" }));
    await waitFor(() =>
      expect(highlight).toHaveBeenCalledWith(
        expect.objectContaining({ language: "ts" }),
        expect.any(Function),
      ),
    );

    fireEvent.click(screen.getByRole("button", { name: "Rust" }));
    await waitFor(() =>
      expect(highlight).toHaveBeenCalledWith(
        expect.objectContaining({ language: "rust" }),
        expect.any(Function),
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

  it("keeps long code scrollable within the responsive block", async () => {
    const { container } = render(<CodeBlock samples={[samples[0]]} />);
    await waitFor(() => expect(highlight).toHaveBeenCalled());

    expect(container.querySelector("pre")).toHaveClass("overflow-x-auto");
    expect(screen.getByText("1")).toHaveClass("hidden", "sm:table-cell");
  });
});
