import { fireEvent, render, screen } from "@testing-library/react";
import { InitialFilesPreview } from "@/components/files/initial-files-preview";
import type { InitialFile } from "@/lib/api/types";

jest.mock("streamdown", () => ({
  Streamdown: ({ children }: { children: string }) => (
    <pre data-testid="streamdown-mock">{children}</pre>
  ),
}));

jest.mock("@streamdown/code", () => ({
  code: {},
}));

describe("InitialFilesPreview", () => {
  it("renders empty state when no files are configured", () => {
    render(<InitialFilesPreview files={[]} />);

    expect(screen.getByText("No initial files configured.")).toBeInTheDocument();
  });

  // Regression: agents/harnesses persisted before `initial_files` existed (or stripped
  // by serializers) hand the component `undefined`. It must not throw "t is not iterable".
  it.each([undefined, null])("renders empty state when files prop is %p", (files) => {
    render(<InitialFilesPreview files={files as InitialFile[] | null | undefined} />);

    expect(screen.getByText("No initial files configured.")).toBeInTheDocument();
  });

  it("shows text file contents inline and lets the user switch files", () => {
    const files: InitialFile[] = [
      {
        path: "/notes.txt",
        content: "plain text preview",
        encoding: "text",
        is_readonly: false,
      },
      {
        path: "/src/app.ts",
        content: "export const value = 1;",
        encoding: "text",
        is_readonly: true,
      },
    ];

    render(<InitialFilesPreview files={files} />);

    expect(screen.getByText("plain text preview")).toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: /\/src\/app\.ts/i }));

    expect(screen.getByText(/export const value = 1;/)).toBeInTheDocument();
    expect(screen.getByText("read-only")).toBeInTheDocument();
  });

  it("shows a binary fallback when inline preview is unavailable", () => {
    const files: InitialFile[] = [
      {
        path: "/assets/archive.bin",
        content: "SGVsbG8=",
        encoding: "base64",
        is_readonly: false,
      },
    ];

    render(<InitialFilesPreview files={files} />);

    expect(screen.getByText("Binary file")).toBeInTheDocument();
    expect(
      screen.getByText(/stored as base64 and can be copied into new sessions/i),
    ).toBeInTheDocument();
  });
});
