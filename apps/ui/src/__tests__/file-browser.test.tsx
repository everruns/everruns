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

// Mock the file hooks
const mockRefetch = jest.fn().mockResolvedValue({ data: [] });
const mockCreateFile = jest.fn();
const mockCreateDir = jest.fn();
const mockDeleteFile = jest.fn();

jest.mock("@/hooks/use-session-files", () => ({
  useFiles: jest.fn(() => ({
    data: [],
    isLoading: false,
    refetch: mockRefetch,
  })),
  useCreateFile: jest.fn(() => ({ mutateAsync: mockCreateFile })),
  useCreateDirectory: jest.fn(() => ({ mutateAsync: mockCreateDir })),
  useDeleteFile: jest.fn(() => ({ mutateAsync: mockDeleteFile })),
}));

// Mock the session-files utilities
const mockListFiles = jest.fn();
jest.mock("@/lib/api/session-files", () => ({
  formatFileSize: (bytes: number) => `${bytes} B`,
  joinPath: (base: string, name: string) => `${base}/${name}`,
  listFiles: (...args: unknown[]) => mockListFiles(...args),
}));

import { FileBrowser } from "@/components/files/file-browser";
import { useFiles } from "@/hooks/use-session-files";

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

// ============================================
// Loading and Empty States
// ============================================

describe("FileBrowser States", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("shows loading state with spinner", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: undefined,
      isLoading: true,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTestId("loader-icon")).toBeInTheDocument();
    expect(screen.getByText("Loading files...")).toBeInTheDocument();
  });

  it("shows empty workspace message when no files", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("Empty workspace")).toBeInTheDocument();
    expect(screen.getByText("Create a file or folder to get started")).toBeInTheDocument();
  });
});

// ============================================
// Toolbar Tests
// ============================================

describe("FileBrowser Toolbar", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (useFiles as jest.Mock).mockReturnValue({
      data: [],
      isLoading: false,
      refetch: mockRefetch,
    });
  });

  it("renders refresh button", () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTitle("Refresh")).toBeInTheDocument();
    expect(screen.getByText("Refresh")).toBeInTheDocument();
  });

  it("renders new folder button", () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTitle("New folder")).toBeInTheDocument();
    expect(screen.getByText("Folder")).toBeInTheDocument();
  });

  it("renders new file button", () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTitle("New file")).toBeInTheDocument();
    expect(screen.getByText("File")).toBeInTheDocument();
  });

  it("calls refetch when refresh button is clicked", async () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    fireEvent.click(screen.getByTitle("Refresh"));
    await waitFor(() => {
      expect(mockRefetch).toHaveBeenCalled();
    });
  });
});

// ============================================
// File Tree Tests
// ============================================

describe("FileBrowser FileTree", () => {
  const mockFile = {
    id: "1",
    session_id: "test-session",
    path: "/workspace/test.ts",
    name: "test.ts",
    is_directory: false,
    is_readonly: false,
    size_bytes: 1024,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  };

  const mockDirectory = {
    id: "2",
    session_id: "test-session",
    path: "/workspace/src",
    name: "src",
    is_directory: true,
    is_readonly: false,
    size_bytes: 0,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  };

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("renders files with name and size", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockFile],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("test.ts")).toBeInTheDocument();
    expect(screen.getByText("1024 B")).toBeInTheDocument();
  });

  it("renders directories", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("src")).toBeInTheDocument();
  });

  it("sorts directories before files", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockFile, mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    const items = screen.getAllByRole("treeitem");
    // Directory should come first
    expect(items[0]).toHaveTextContent("src");
  });

  it("calls onFileSelect when file is clicked", () => {
    const onFileSelect = jest.fn();
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockFile],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" onFileSelect={onFileSelect} />);

    fireEvent.click(screen.getByText("test.ts"));
    expect(onFileSelect).toHaveBeenCalledWith(mockFile);
  });

  it("does not call onFileSelect when directory is clicked", () => {
    const onFileSelect = jest.fn();
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" onFileSelect={onFileSelect} />);

    fireEvent.click(screen.getByText("src"));
    expect(onFileSelect).not.toHaveBeenCalled();
  });

  it("shows lock icon for readonly files", () => {
    const readonlyFile = { ...mockFile, is_readonly: true };
    (useFiles as jest.Mock).mockReturnValue({
      data: [readonlyFile],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTestId("lock-icon")).toBeInTheDocument();
  });

  it("does not show delete button for readonly files", () => {
    const readonlyFile = { ...mockFile, is_readonly: true };
    (useFiles as jest.Mock).mockReturnValue({
      data: [readonlyFile],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    // Hover over the file to reveal actions
    const fileItem = screen.getByText("test.ts").closest("[role='treeitem']");
    fireEvent.mouseEnter(fileItem!);

    // Should not have trash icon for readonly file
    expect(screen.queryByTitle("Delete")).not.toBeInTheDocument();
  });

  it("highlights selected file", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockFile],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" selectedPath="/workspace/test.ts" />);

    const selectedItem = screen.getByText("test.ts").closest("[role='treeitem']");
    expect(selectedItem).toHaveClass("bg-accent/20");
  });
});

// ============================================
// File Type Icons
// ============================================

describe("FileBrowser File Icons", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  const testFileIcon = (filename: string, expectedIconTestId: string) => {
    const file = {
      id: "1",
      session_id: "test-session",
      path: `/workspace/${filename}`,
      name: filename,
      is_directory: false,
      is_readonly: false,
      size_bytes: 100,
      created_at: "2024-01-01T00:00:00Z",
      updated_at: "2024-01-01T00:00:00Z",
    };

    (useFiles as jest.Mock).mockReturnValue({
      data: [file],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTestId(expectedIconTestId)).toBeInTheDocument();
  };

  it("shows code icon for TypeScript files", () => {
    testFileIcon("app.ts", "file-code-icon");
  });

  it("shows code icon for TSX files", () => {
    testFileIcon("component.tsx", "file-code-icon");
  });

  it("shows JSON icon for JSON files", () => {
    testFileIcon("package.json", "file-json-icon");
  });

  it("shows text icon for markdown files", () => {
    testFileIcon("README.md", "file-text-icon");
  });

  it("shows image icon for PNG files", () => {
    testFileIcon("logo.png", "image-icon");
  });
});

// ============================================
// Create File Dialog
// Note: Dialog tests are skipped as they require complex Radix UI portal mocking.
// The dialog functionality is covered by E2E tests.
// ============================================

describe("FileBrowser Create File Dialog", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (useFiles as jest.Mock).mockReturnValue({
      data: [],
      isLoading: false,
      refetch: mockRefetch,
    });
    mockCreateFile.mockResolvedValue({});
  });

  it("renders new file button in toolbar", () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTitle("New file")).toBeInTheDocument();
  });
});

// ============================================
// Create Folder Dialog
// Note: Dialog tests are skipped as they require complex Radix UI portal mocking.
// ============================================

describe("FileBrowser Create Folder Dialog", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    (useFiles as jest.Mock).mockReturnValue({
      data: [],
      isLoading: false,
      refetch: mockRefetch,
    });
    mockCreateDir.mockResolvedValue({});
  });

  it("renders new folder button in toolbar", () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByTitle("New folder")).toBeInTheDocument();
  });
});

// ============================================
// Delete Functionality
// ============================================

describe("FileBrowser Delete", () => {
  const mockFile = {
    id: "1",
    session_id: "test-session",
    path: "/workspace/test.ts",
    name: "test.ts",
    is_directory: false,
    is_readonly: false,
    size_bytes: 1024,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  };

  beforeEach(() => {
    jest.clearAllMocks();
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockFile],
      isLoading: false,
      refetch: mockRefetch,
    });
    mockDeleteFile.mockResolvedValue({});
    window.confirm = jest.fn(() => true);
  });

  it("shows delete button on hover", () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    // The delete button should exist but be hidden until hover
    const deleteButton = screen.getByTitle("Delete");
    expect(deleteButton).toBeInTheDocument();
  });

  it("confirms before deleting", async () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    fireEvent.click(screen.getByTitle("Delete"));

    expect(window.confirm).toHaveBeenCalledWith('Delete file "test.ts"?');
  });

  it("deletes file when confirmed", async () => {
    renderWithProviders(<FileBrowser sessionId="test-session" />);

    fireEvent.click(screen.getByTitle("Delete"));

    await waitFor(() => {
      expect(mockDeleteFile).toHaveBeenCalledWith({
        sessionId: "test-session",
        path: "/workspace/test.ts",
        recursive: false,
      });
    });
  });

  it("does not delete when cancelled", async () => {
    (window.confirm as jest.Mock).mockReturnValue(false);

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    fireEvent.click(screen.getByTitle("Delete"));

    expect(mockDeleteFile).not.toHaveBeenCalled();
  });

  // Note: Directory delete test requires UI improvement - folder actions are currently
  // inside CollapsibleContent which is hidden when collapsed. This should be addressed
  // by adding an actions prop to FileTreeFolder that renders in the trigger row.
});

// ============================================
// Directory Expansion
// ============================================

describe("FileBrowser Directory Expansion", () => {
  const mockDirectory = {
    id: "1",
    session_id: "test-session",
    path: "/src",
    name: "src",
    is_directory: true,
    is_readonly: false,
    size_bytes: 0,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  };

  const mockSubFile = {
    id: "2",
    session_id: "test-session",
    path: "/src/index.ts",
    name: "index.ts",
    is_directory: false,
    is_readonly: false,
    size_bytes: 500,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  };

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("calls listFiles API when directory is expanded", async () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    mockListFiles.mockResolvedValue([mockSubFile]);

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    // Click to expand directory
    fireEvent.click(screen.getByText("src"));

    await waitFor(() => {
      expect(mockListFiles).toHaveBeenCalledWith("test-session", "/src");
    });
  });

  it("renders subdirectory files after expansion", async () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    mockListFiles.mockResolvedValue([mockSubFile]);

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    fireEvent.click(screen.getByText("src"));

    await waitFor(() => {
      expect(screen.getByText("index.ts")).toBeInTheDocument();
    });
  });

  it("reloads expanded directories on refresh", async () => {
    mockRefetch.mockResolvedValue({ data: [mockDirectory] });

    (useFiles as jest.Mock).mockReturnValue({
      data: [mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    mockListFiles.mockResolvedValue([mockSubFile]);

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    // Expand directory first
    fireEvent.click(screen.getByText("src"));
    await waitFor(() => {
      expect(mockListFiles).toHaveBeenCalledWith("test-session", "/src");
    });

    mockListFiles.mockClear();
    mockListFiles.mockResolvedValue([mockSubFile]);

    // Click refresh
    fireEvent.click(screen.getByTitle("Refresh"));

    await waitFor(() => {
      expect(mockRefetch).toHaveBeenCalled();
      expect(mockListFiles).toHaveBeenCalledWith("test-session", "/src");
    });
  });
});

// ============================================
// Recursion Guard & Edge Cases
// ============================================

describe("FileBrowser recursion guard", () => {
  const makeFile = (
    overrides: Partial<{
      id: string;
      path: string;
      name: string;
      is_directory: boolean;
      is_readonly: boolean;
      size_bytes: number;
    }>,
  ) => ({
    id: overrides.id ?? "f1",
    session_id: "test-session",
    path: overrides.path ?? "/workspace/file.txt",
    name: overrides.name ?? "file.txt",
    is_directory: overrides.is_directory ?? false,
    is_readonly: overrides.is_readonly ?? false,
    size_bytes: overrides.size_bytes ?? 0,
    created_at: "2024-01-01T00:00:00Z",
    updated_at: "2024-01-01T00:00:00Z",
  });

  beforeEach(() => {
    jest.clearAllMocks();
  });

  it("does not stack-overflow when a directory listing contains a self-referencing entry", () => {
    // This is the exact bug: API returns an entry whose path equals the parent dir
    const selfRef = makeFile({
      id: "self",
      path: "/workspace",
      name: "workspace",
      is_directory: true,
    });
    const normalFile = makeFile({
      id: "ok",
      path: "/workspace/hello.ts",
      name: "hello.ts",
    });

    (useFiles as jest.Mock).mockReturnValue({
      data: [selfRef, normalFile],
      isLoading: false,
      refetch: mockRefetch,
    });

    // Before the fix this would throw RangeError: Maximum call stack size exceeded
    expect(() => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);
    }).not.toThrow();

    // The normal file should still render
    expect(screen.getByText("hello.ts")).toBeInTheDocument();
    // The self-referencing entry should be filtered out
    expect(screen.queryByText("workspace")).not.toBeInTheDocument();
  });

  it("filters self-referencing entries but keeps legitimate children", () => {
    const dirA = makeFile({
      id: "dirA",
      path: "/workspace/src",
      name: "src",
      is_directory: true,
    });
    const fileB = makeFile({
      id: "fileB",
      path: "/workspace/README.md",
      name: "README.md",
    });
    // Pathological: an entry whose path matches the root listing path
    const selfRef = makeFile({
      id: "selfRef",
      path: "/workspace",
      name: ".",
      is_directory: true,
    });

    (useFiles as jest.Mock).mockReturnValue({
      data: [dirA, fileB, selfRef],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("src")).toBeInTheDocument();
    expect(screen.getByText("README.md")).toBeInTheDocument();
  });

  it("does not stack-overflow with mutual directory cycle (A lists B, B lists A)", async () => {
    const dirA = makeFile({
      id: "a",
      path: "/workspace/a",
      name: "a",
      is_directory: true,
    });
    const dirBInsideA = makeFile({
      id: "b-in-a",
      path: "/workspace/a/b",
      name: "b",
      is_directory: true,
    });
    // Cycle: B's listing points back to A's path
    const dirAInsideB = makeFile({
      id: "a-in-b",
      path: "/workspace/a",
      name: "a",
      is_directory: true,
    });

    (useFiles as jest.Mock).mockReturnValue({
      data: [dirA],
      isLoading: false,
      refetch: mockRefetch,
    });

    // When dir A is expanded it returns B; when B is expanded it returns A (cycle)
    mockListFiles.mockImplementation((_sid: string, path: string) => {
      if (path === "/workspace/a") return Promise.resolve([dirBInsideA]);
      if (path === "/workspace/a/b") return Promise.resolve([dirAInsideB]);
      return Promise.resolve([]);
    });

    expect(() => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);
    }).not.toThrow();

    // Expand dir A
    fireEvent.click(screen.getAllByText("a")[0]);

    await waitFor(() => {
      expect(screen.getByText("b")).toBeInTheDocument();
    });

    // Expand dir B — dirAInsideB.path === "/workspace/a" which is already loaded,
    // so the depth guard is the safety net against infinite recursive rendering
    fireEvent.click(screen.getByText("b"));

    await waitFor(() => {
      expect(mockListFiles).toHaveBeenCalledWith("test-session", "/workspace/a/b");
    });

    // Should have rendered without throwing (depth guard caps recursion)
    // Multiple "a" nodes appear because the cycle re-renders A's subtree
    // until the depth limit is hit — that's the expected safe behavior
    expect(screen.getAllByText("a").length).toBeGreaterThanOrEqual(1);
  });

  it("renders empty directory without errors", () => {
    const emptyDir = makeFile({
      id: "empty",
      path: "/workspace/empty",
      name: "empty",
      is_directory: true,
    });

    (useFiles as jest.Mock).mockReturnValue({
      data: [emptyDir],
      isLoading: false,
      refetch: mockRefetch,
    });

    expect(() => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);
    }).not.toThrow();

    expect(screen.getByText("empty")).toBeInTheDocument();
  });

  it("handles directory listing with only self-referencing entries (all filtered)", () => {
    // Every entry in the root listing points back to /workspace
    const selfRef1 = makeFile({
      id: "s1",
      path: "/workspace",
      name: ".",
      is_directory: true,
    });
    const selfRef2 = makeFile({
      id: "s2",
      path: "/workspace",
      name: "..",
      is_directory: true,
    });

    (useFiles as jest.Mock).mockReturnValue({
      data: [selfRef1, selfRef2],
      isLoading: false,
      refetch: mockRefetch,
    });

    // All entries filtered → falls through to empty workspace
    expect(() => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);
    }).not.toThrow();
  });

  it("handles duplicate file entries gracefully", () => {
    const file1 = makeFile({
      id: "dup1",
      path: "/workspace/dup.ts",
      name: "dup.ts",
    });
    const file2 = makeFile({
      id: "dup2",
      path: "/workspace/dup.ts",
      name: "dup.ts",
    });

    (useFiles as jest.Mock).mockReturnValue({
      data: [file1, file2],
      isLoading: false,
      refetch: mockRefetch,
    });

    expect(() => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);
    }).not.toThrow();

    // Both render (keyed by unique id)
    expect(screen.getAllByText("dup.ts")).toHaveLength(2);
  });

  it("sorts directories before files even when self-referencing entries are present", () => {
    const selfRef = makeFile({
      id: "self",
      path: "/workspace",
      name: "workspace",
      is_directory: true,
    });
    const dirZ = makeFile({
      id: "dirZ",
      path: "/workspace/z-dir",
      name: "z-dir",
      is_directory: true,
    });
    const fileA = makeFile({
      id: "fileA",
      path: "/workspace/a-file.ts",
      name: "a-file.ts",
    });

    (useFiles as jest.Mock).mockReturnValue({
      data: [fileA, selfRef, dirZ],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    const items = screen.getAllByRole("treeitem");
    // Directory "z-dir" should come before file "a-file.ts" despite alphabetical order
    expect(items[0]).toHaveTextContent("z-dir");
    expect(items[1]).toHaveTextContent("a-file.ts");
  });
});
