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
const mockRefetch = jest.fn();
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
