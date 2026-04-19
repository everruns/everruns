import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { InitialFilesEditor } from "@/components/initial-files-editor";
import type { InitialFile } from "@/lib/api/types";

const originalError = console.error;
beforeAll(() => {
  console.error = (...args: unknown[]) => {
    if (typeof args[0] === "string" && args[0].includes("not wrapped in act")) {
      return;
    }
    originalError.call(console, ...args);
  };
});

afterAll(() => {
  console.error = originalError;
});

jest.mock("@/components/files/file-previews", () => ({
  FilePreview: ({
    content,
    extension,
    encoding,
  }: {
    content: string;
    extension: string;
    encoding: string;
  }) => <div data-testid="file-preview">{`${extension}:${encoding}:${content}`}</div>,
  canPreview: (extension: string, encoding: string) => encoding === "base64" && extension === "png",
  getPreviewType: (extension: string, encoding: string) => {
    if (encoding === "base64") {
      return extension === "png" ? "image" : "binary";
    }
    return "text";
  },
}));

jest.mock("@/lib/api/session-files", () => ({
  formatFileSize: (bytes: number) => `${bytes} B`,
}));

function TestHarness({ initialFiles = [] }: { initialFiles?: InitialFile[] }) {
  const [files, setFiles] = useState(initialFiles);

  return (
    <>
      <InitialFilesEditor value={files} onChange={setFiles} />
      <pre data-testid="files-state">{JSON.stringify(files)}</pre>
    </>
  );
}

function readFilesState(): InitialFile[] {
  const raw = screen.getByTestId("files-state").textContent ?? "[]";
  return JSON.parse(raw) as InitialFile[];
}

describe("InitialFilesEditor", () => {
  it("shows an empty state when no files are configured", () => {
    render(<TestHarness />);

    expect(screen.getByText(/No starter files configured/i)).toBeInTheDocument();
  });

  it("adds a new text file and selects it", () => {
    render(<TestHarness />);

    fireEvent.click(screen.getByRole("button", { name: "Add Text File" }));

    expect(screen.getByDisplayValue("/file-1.txt")).toBeInTheDocument();
    expect(readFilesState()).toEqual([
      {
        path: "/file-1.txt",
        content: "",
        encoding: "text",
        is_readonly: false,
      },
    ]);
  });

  it("updates file path, content, and readonly state", () => {
    render(
      <TestHarness
        initialFiles={[
          {
            path: "/notes.txt",
            content: "hello",
            encoding: "text",
            is_readonly: false,
          },
        ]}
      />,
    );

    fireEvent.change(screen.getByLabelText("Path"), {
      target: { value: "/docs/notes.md" },
    });
    fireEvent.click(screen.getByRole("checkbox", { name: "Read-only in session" }));
    fireEvent.change(screen.getByLabelText("Content"), {
      target: { value: "updated" },
    });

    expect(readFilesState()).toEqual([
      {
        path: "/docs/notes.md",
        content: "updated",
        encoding: "text",
        is_readonly: true,
      },
    ]);
  });

  it("renders the filesystem tree and switches selection", () => {
    render(
      <TestHarness
        initialFiles={[
          {
            path: "/docs/readme.md",
            content: "# hello",
            encoding: "text",
            is_readonly: false,
          },
          {
            path: "/logo.png",
            content: "Zm9v",
            encoding: "base64",
            is_readonly: false,
          },
        ]}
      />,
    );

    expect(screen.getByText("docs")).toBeInTheDocument();
    expect(screen.getAllByText("readme.md")).not.toHaveLength(0);

    fireEvent.click(screen.getByText("logo.png"));

    expect(screen.getByDisplayValue("/logo.png")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Preview" })).toBeInTheDocument();
  });

  it("shows the preview pane for previewable files", () => {
    render(
      <TestHarness
        initialFiles={[
          {
            path: "/logo.png",
            content: "Zm9v",
            encoding: "base64",
            is_readonly: false,
          },
        ]}
      />,
    );

    fireEvent.click(screen.getByRole("button", { name: "Preview" }));

    expect(screen.getByTestId("file-preview")).toHaveTextContent("png:base64:Zm9v");
  });
});
