import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

// Suppress act() warnings for async state updates in this file
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

// Mock lucide-react icons
jest.mock("lucide-react", () => ({
  ChevronRightIcon: () => <span data-testid="chevron-icon" />,
  Plus: () => <span data-testid="plus-icon" />,
  FolderPlus: () => <span data-testid="folder-plus-icon" />,
  Trash2: () => <span data-testid="trash-icon" />,
  RefreshCw: () => <span data-testid="refresh-icon" />,
  Lock: () => <span data-testid="lock-icon" />,
  FileCode: () => <span data-testid="file-code-icon" />,
  FileText: () => <span data-testid="file-text-icon" />,
  FileJson: () => <span data-testid="file-json-icon" />,
  Image: () => <span data-testid="image-icon" />,
  File: () => <span data-testid="file-icon" />,
  Loader2: () => <span data-testid="loader-icon" />,
  FileIcon: () => <span data-testid="file-icon" />,
  FolderIcon: () => <span data-testid="folder-icon" />,
  FolderOpenIcon: () => <span data-testid="folder-open-icon" />,
}));

import { FileBrowser } from "@/components/files/file-browser";
import type { FileSource } from "@/components/files/file-source";
import type { FileInfo } from "@/lib/api/types";

// A fake filesystem: `list` answers the root from `rootFiles` and any other
// folder from `mockListFiles`; writes are present unless `readOnly`.
let rootFiles: FileInfo[] | Promise<FileInfo[]> = [];
const mockListFiles = jest.fn();
const mockCreateFile = jest.fn();
const mockMkdir = jest.fn();
const mockRemove = jest.fn();

function makeSource({ readOnly = false }: { readOnly?: boolean } = {}): FileSource {
  return {
    key: "workspace:test-session",
    root: "/workspace",
    list: jest.fn((path: string) =>
      path === "/workspace" ? Promise.resolve(rootFiles) : mockListFiles(path),
    ),
    read: jest.fn(),
    downloadUrl: (path: string) => `/download${path}`,
    writes: readOnly
      ? undefined
      : {
          createFile: mockCreateFile,
          updateFile: jest.fn(),
          mkdir: mockMkdir,
          remove: mockRemove,
        },
  };
}

const createQueryClient = () =>
  new QueryClient({
    defaultOptions: {
      queries: { retry: false },
    },
  });

const renderWithProviders = (ui: React.ReactElement) => {
  const queryClient = createQueryClient();
  return render(<QueryClientProvider client={queryClient}>{ui}</QueryClientProvider>);
};

const entry = (overrides: Partial<FileInfo>): FileInfo =>
  ({
    id: overrides.id ?? overrides.path ?? "f1",
    session_id: "test-session",
    path: "/workspace/file.txt",
    name: "file.txt",
    is_directory: false,
    is_readonly: false,
    size_bytes: 0,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
    ...overrides,
  }) as FileInfo;

const mockFile = entry({ id: "1", path: "/workspace/test.ts", name: "test.ts", size_bytes: 1024 });
const mockDirectory = entry({
  id: "2",
  path: "/workspace/src",
  name: "src",
  is_directory: true,
});

beforeEach(() => {
  jest.clearAllMocks();
  rootFiles = [];
});

// ============================================
// Loading and Empty States
// ============================================

describe("FileBrowser States", () => {
  it("shows loading state with spinner", () => {
    rootFiles = new Promise(() => {});
    renderWithProviders(<FileBrowser source={makeSource()} />);

    expect(screen.getByTestId("loader-icon")).toBeInTheDocument();
    expect(screen.getByText("Loading files...")).toBeInTheDocument();
  });

  it("shows empty workspace message when no files", async () => {
    renderWithProviders(<FileBrowser source={makeSource()} />);

    expect(await screen.findByText("No files yet")).toBeInTheDocument();
    expect(screen.getByText("Create a file or folder to get started")).toBeInTheDocument();
  });

  it("renders a source's own empty state", async () => {
    renderWithProviders(
      <FileBrowser source={makeSource()} emptyState={<p>Nothing synced yet</p>} />,
    );

    expect(await screen.findByText("Nothing synced yet")).toBeInTheDocument();
  });
});

// ============================================
// Toolbar Tests
// ============================================

describe("FileBrowser Toolbar", () => {
  it("lists the root again when refresh is clicked", async () => {
    const source = makeSource();
    renderWithProviders(<FileBrowser source={source} />);
    await screen.findByText("No files yet");

    fireEvent.click(screen.getByTitle("Refresh"));
    await waitFor(() => {
      expect(source.list).toHaveBeenCalledTimes(2);
    });
  });

  it("offers create actions only for a writable source", async () => {
    const { unmount } = renderWithProviders(<FileBrowser source={makeSource()} />);
    await screen.findByText("No files yet");
    expect(screen.getByTitle("New folder")).toBeInTheDocument();
    expect(screen.getByTitle("New file")).toBeInTheDocument();
    unmount();

    renderWithProviders(<FileBrowser source={makeSource({ readOnly: true })} />);
    await screen.findByText("No files yet");
    expect(screen.queryByTitle("New folder")).not.toBeInTheDocument();
    expect(screen.queryByTitle("New file")).not.toBeInTheDocument();
  });

  it("hides new folder when the source has no folders of its own", async () => {
    const source = makeSource();
    delete source.writes!.mkdir;
    renderWithProviders(<FileBrowser source={source} />);
    await screen.findByText("No files yet");

    expect(screen.queryByTitle("New folder")).not.toBeInTheDocument();
    expect(screen.getByTitle("New file")).toBeInTheDocument();
  });
});

// ============================================
// File Tree Tests
// ============================================

describe("FileBrowser FileTree", () => {
  it("calls onFileSelect when file is clicked", async () => {
    const onFileSelect = jest.fn();
    rootFiles = [mockFile];

    renderWithProviders(<FileBrowser source={makeSource()} onFileSelect={onFileSelect} />);

    fireEvent.click(await screen.findByText("test.ts"));
    expect(onFileSelect).toHaveBeenCalledWith(mockFile);
  });

  it("does not call onFileSelect when directory is clicked", async () => {
    const onFileSelect = jest.fn();
    rootFiles = [mockDirectory];
    mockListFiles.mockResolvedValue([]);

    renderWithProviders(<FileBrowser source={makeSource()} onFileSelect={onFileSelect} />);

    fireEvent.click(await screen.findByText("src"));
    expect(onFileSelect).not.toHaveBeenCalled();
  });

  it("shows lock icon for readonly files in a writable source", async () => {
    rootFiles = [{ ...mockFile, is_readonly: true }];

    renderWithProviders(<FileBrowser source={makeSource()} />);

    expect(await screen.findByTestId("lock-icon")).toBeInTheDocument();
  });

  it("does not show delete button for readonly files", async () => {
    rootFiles = [{ ...mockFile, is_readonly: true }];

    renderWithProviders(<FileBrowser source={makeSource()} />);

    const fileItem = (await screen.findByText("test.ts")).closest("[role='treeitem']");
    fireEvent.mouseEnter(fileItem!);

    expect(screen.queryByTitle("Delete")).not.toBeInTheDocument();
  });

  it("renders a file decoration from the source's page", async () => {
    rootFiles = [mockFile];

    renderWithProviders(
      <FileBrowser
        source={makeSource({ readOnly: true })}
        decorateFile={(file) => <span>{`status of ${file.name}`}</span>}
      />,
    );

    expect(await screen.findByText("status of test.ts")).toBeInTheDocument();
  });
});

// ============================================
// Delete Functionality
// ============================================

describe("FileBrowser Delete", () => {
  beforeEach(() => {
    rootFiles = [mockFile];
    mockRemove.mockResolvedValue({});
    window.confirm = jest.fn(() => true);
  });

  it("confirms before deleting", async () => {
    renderWithProviders(<FileBrowser source={makeSource()} />);

    fireEvent.click(await screen.findByTitle("Delete"));

    expect(window.confirm).toHaveBeenCalledWith('Delete file "test.ts"?');
  });

  it("deletes file when confirmed", async () => {
    renderWithProviders(<FileBrowser source={makeSource()} />);

    fireEvent.click(await screen.findByTitle("Delete"));

    await waitFor(() => {
      expect(mockRemove).toHaveBeenCalledWith("/workspace/test.ts", false);
    });
  });

  it("does not delete when cancelled", async () => {
    (window.confirm as jest.Mock).mockReturnValue(false);

    renderWithProviders(<FileBrowser source={makeSource()} />);

    fireEvent.click(await screen.findByTitle("Delete"));

    expect(mockRemove).not.toHaveBeenCalled();
  });

  it("has no delete button in a read-only source", async () => {
    renderWithProviders(<FileBrowser source={makeSource({ readOnly: true })} />);

    await screen.findByText("test.ts");
    expect(screen.queryByTitle("Delete")).not.toBeInTheDocument();
  });
});

// ============================================
// Directory Expansion
// ============================================

describe("FileBrowser Directory Expansion", () => {
  const subFile = entry({ id: "3", path: "/workspace/src/index.ts", name: "index.ts" });

  it("renders subdirectory files after expansion", async () => {
    rootFiles = [mockDirectory];
    mockListFiles.mockResolvedValue([subFile]);

    renderWithProviders(<FileBrowser source={makeSource()} />);

    fireEvent.click(await screen.findByText("src"));

    expect(await screen.findByText("index.ts")).toBeInTheDocument();
  });

  it("reloads expanded directories on refresh", async () => {
    rootFiles = [mockDirectory];
    mockListFiles.mockResolvedValue([subFile]);

    renderWithProviders(<FileBrowser source={makeSource()} />);

    fireEvent.click(await screen.findByText("src"));
    await waitFor(() => {
      expect(mockListFiles).toHaveBeenCalledWith("/workspace/src");
    });

    mockListFiles.mockClear();
    mockListFiles.mockResolvedValue([subFile]);

    fireEvent.click(screen.getByTitle("Refresh"));

    await waitFor(() => {
      expect(mockListFiles).toHaveBeenCalledWith("/workspace/src");
    });
  });

  it("browses a source rooted somewhere else", async () => {
    const source: FileSource = {
      ...makeSource({ readOnly: true }),
      root: "/",
      list: jest.fn((path: string) =>
        Promise.resolve(
          path === "/"
            ? [entry({ id: "docs", path: "/docs", name: "docs", is_directory: true })]
            : [entry({ id: "guide", path: "/docs/guide.md", name: "guide.md" })],
        ),
      ),
    };
    renderWithProviders(<FileBrowser source={source} />);

    fireEvent.click(await screen.findByText("docs"));

    expect(await screen.findByText("guide.md")).toBeInTheDocument();
    expect(source.list).toHaveBeenCalledWith("/docs");
  });
});

// ============================================
// Recursion Guard & Edge Cases
// ============================================

describe("FileBrowser recursion guard", () => {
  it("does not stack-overflow when a directory listing contains a self-referencing entry", async () => {
    // This is the exact bug: API returns an entry whose path equals the parent dir
    rootFiles = [
      entry({ id: "self", path: "/workspace", name: "workspace", is_directory: true }),
      entry({ id: "ok", path: "/workspace/hello.ts", name: "hello.ts" }),
    ];

    renderWithProviders(<FileBrowser source={makeSource()} />);

    // The normal file should still render
    expect(await screen.findByText("hello.ts")).toBeInTheDocument();
    // The self-referencing entry should be filtered out
    expect(screen.queryByText("workspace")).not.toBeInTheDocument();
  });

  it("does not stack-overflow with mutual directory cycle (A lists B, B lists A)", async () => {
    rootFiles = [entry({ id: "a", path: "/workspace/a", name: "a", is_directory: true })];

    // When dir A is expanded it returns B; when B is expanded it returns A (cycle)
    mockListFiles.mockImplementation((path: string) => {
      if (path === "/workspace/a")
        return Promise.resolve([
          entry({ id: "b-in-a", path: "/workspace/a/b", name: "b", is_directory: true }),
        ]);
      if (path === "/workspace/a/b")
        return Promise.resolve([
          entry({ id: "a-in-b", path: "/workspace/a", name: "a", is_directory: true }),
        ]);
      return Promise.resolve([]);
    });

    renderWithProviders(<FileBrowser source={makeSource()} />);

    fireEvent.click((await screen.findAllByText("a"))[0]);

    fireEvent.click(await screen.findByText("b"));

    await waitFor(() => {
      expect(mockListFiles).toHaveBeenCalledWith("/workspace/a/b");
    });

    // The depth guard caps the recursive render of the cycle.
    expect(screen.getAllByText("a").length).toBeGreaterThanOrEqual(1);
  });

  it("handles duplicate file entries gracefully", async () => {
    rootFiles = [
      entry({ id: "dup1", path: "/workspace/dup.ts", name: "dup.ts" }),
      entry({ id: "dup2", path: "/workspace/dup.ts", name: "dup.ts" }),
    ];

    renderWithProviders(<FileBrowser source={makeSource()} />);

    // Both render (keyed by unique id)
    expect(await screen.findAllByText("dup.ts")).toHaveLength(2);
  });

  it("sorts directories before files even when self-referencing entries are present", async () => {
    rootFiles = [
      entry({ id: "fileA", path: "/workspace/a-file.ts", name: "a-file.ts" }),
      entry({ id: "self", path: "/workspace", name: "workspace", is_directory: true }),
      entry({ id: "dirZ", path: "/workspace/z-dir", name: "z-dir", is_directory: true }),
    ];

    renderWithProviders(<FileBrowser source={makeSource()} />);

    await screen.findByText("a-file.ts");
    const items = screen.getAllByRole("treeitem");
    expect(items[0]).toHaveTextContent("z-dir");
    expect(items[1]).toHaveTextContent("a-file.ts");
  });
});
