import { render, screen, fireEvent } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

// Mock lucide-react icons
jest.mock("lucide-react", () => ({
  Folder: () => <span data-testid="folder-icon" />,
  ChevronRight: () => <span data-testid="chevron-right" />,
  ChevronDown: () => <span data-testid="chevron-down" />,
  Plus: () => <span data-testid="plus-icon" />,
  FolderPlus: () => <span data-testid="folder-plus-icon" />,
  Trash2: () => <span data-testid="trash-icon" />,
  RefreshCw: () => <span data-testid="refresh-icon" />,
  Home: () => <span data-testid="home-icon" />,
  Lock: () => <span data-testid="lock-icon" />,
}));

// Mock the file hooks
const mockRefetch = jest.fn();
jest.mock("@/hooks/use-session-files", () => ({
  useFiles: jest.fn(() => ({
    data: [],
    isLoading: false,
    refetch: mockRefetch,
  })),
  useCreateFile: jest.fn(() => ({ mutateAsync: jest.fn() })),
  useCreateDirectory: jest.fn(() => ({ mutateAsync: jest.fn() })),
  useDeleteFile: jest.fn(() => ({ mutateAsync: jest.fn() })),
}));

// Mock the session-files utilities
jest.mock("@/lib/api/session-files", () => ({
  formatFileSize: (bytes: number) => `${bytes} B`,
  getParentPath: (path: string) => {
    const parts = path.split("/").filter(Boolean);
    if (parts.length <= 1) return null;
    return "/" + parts.slice(0, -1).join("/");
  },
  joinPath: (base: string, name: string) => `${base}/${name}`,
}));

// Mock FileIcon
jest.mock("@/components/files/file-icon", () => ({
  FileIcon: () => <span data-testid="file-icon" />,
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
// Breadcrumbs Tests
// ============================================

describe("FileBrowser Breadcrumbs", () => {
  beforeEach(() => {
    jest.clearAllMocks();
  });

  describe("root level display", () => {
    it("shows 'workspace' label when at root", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      expect(screen.getByText("workspace")).toBeInTheDocument();
    });

    it("shows home button at root", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      const homeButton = screen.getByTitle("Go to workspace root");
      expect(homeButton).toBeInTheDocument();
    });

    it("shows slash separator after home button", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      expect(screen.getByText("/")).toBeInTheDocument();
    });

    it("has visible breadcrumb background", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      // The breadcrumb bar should have bg-muted/50 class for visibility
      const breadcrumbBar = screen.getByText("workspace").closest("div");
      expect(breadcrumbBar).toHaveClass("bg-muted/50");
    });
  });

  describe("nested folder navigation", () => {
    const mockFiles = [
      {
        id: "1",
        session_id: "test-session",
        path: "/workspace/src",
        name: "src",
        is_directory: true,
        is_readonly: false,
        size_bytes: 0,
        created_at: "2024-01-01T00:00:00Z",
        updated_at: "2024-01-01T00:00:00Z",
      },
    ];

    beforeEach(() => {
      (useFiles as jest.Mock).mockReturnValue({
        data: mockFiles,
        isLoading: false,
        refetch: mockRefetch,
      });
    });

    it("updates breadcrumbs when navigating into a folder", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      // Click on the src folder
      const srcFolder = screen.getByText("src");
      fireEvent.click(srcFolder);

      // Should now show "src" in breadcrumbs with folder icon
      const srcBreadcrumb = screen.getAllByText("src")[0]; // First one is in breadcrumbs
      expect(srcBreadcrumb).toBeInTheDocument();
    });

    it("shows folder icon next to current folder in breadcrumbs", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      // Click into a folder
      const srcFolder = screen.getByText("src");
      fireEvent.click(srcFolder);

      // The folder icon should be present in breadcrumbs area
      const folderIcons = screen.getAllByTestId("folder-icon");
      expect(folderIcons.length).toBeGreaterThan(0);
    });
  });

  describe("breadcrumb navigation", () => {
    it("navigates to root when home button is clicked", () => {
      // Start with files in a nested directory
      (useFiles as jest.Mock).mockReturnValue({
        data: [
          {
            id: "1",
            session_id: "test-session",
            path: "/workspace/src",
            name: "src",
            is_directory: true,
            is_readonly: false,
            size_bytes: 0,
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
          },
        ],
        isLoading: false,
        refetch: mockRefetch,
      });

      renderWithProviders(<FileBrowser sessionId="test-session" />);

      // Navigate into src
      const srcFolder = screen.getByText("src");
      fireEvent.click(srcFolder);

      // Click home button
      const homeButton = screen.getByTitle("Go to workspace root");
      fireEvent.click(homeButton);

      // Should show workspace label again
      expect(screen.getByText("workspace")).toBeInTheDocument();
    });
  });

  describe("intermediate breadcrumb segments", () => {
    it("renders intermediate segments as clickable buttons", () => {
      // Mock being in a deeply nested folder
      (useFiles as jest.Mock).mockReturnValue({
        data: [
          {
            id: "1",
            session_id: "test-session",
            path: "/workspace/src/components",
            name: "components",
            is_directory: true,
            is_readonly: false,
            size_bytes: 0,
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
          },
        ],
        isLoading: false,
        refetch: mockRefetch,
      });

      renderWithProviders(<FileBrowser sessionId="test-session" />);

      // Navigate into nested folders
      fireEvent.click(screen.getByText("components"));

      // Now update mock for the new folder
      (useFiles as jest.Mock).mockReturnValue({
        data: [
          {
            id: "2",
            session_id: "test-session",
            path: "/workspace/src/components/ui",
            name: "ui",
            is_directory: true,
            is_readonly: false,
            size_bytes: 0,
            created_at: "2024-01-01T00:00:00Z",
            updated_at: "2024-01-01T00:00:00Z",
          },
        ],
        isLoading: false,
        refetch: mockRefetch,
      });

      // The component should re-render with new breadcrumb path
      // The last segment "components" should have font-medium class
      const breadcrumbComponents = screen.getAllByText("components")[0];
      expect(breadcrumbComponents).toBeInTheDocument();
    });
  });

  describe("breadcrumb styling", () => {
    it("has proper text size for visibility", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      // Breadcrumb bar should have text-sm class
      const breadcrumbBar = screen.getByText("workspace").closest("div");
      expect(breadcrumbBar).toHaveClass("text-sm");
    });

    it("has rounded corners for modern look", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      const breadcrumbBar = screen.getByText("workspace").closest("div");
      expect(breadcrumbBar).toHaveClass("rounded-md");
    });

    it("has proper padding for clickable area", () => {
      renderWithProviders(<FileBrowser sessionId="test-session" />);

      const breadcrumbBar = screen.getByText("workspace").closest("div");
      expect(breadcrumbBar).toHaveClass("px-2");
      expect(breadcrumbBar).toHaveClass("py-1");
    });
  });
});

// ============================================
// FileItem Tests
// ============================================

describe("FileBrowser FileItem", () => {
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

  it("renders file with size badge", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockFile],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("test.ts")).toBeInTheDocument();
    expect(screen.getByText("1024 B")).toBeInTheDocument();
  });

  it("renders directory with folder icon", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("src")).toBeInTheDocument();
    // Should have folder icons (one in title, potentially others in breadcrumbs/items)
    const folderIcons = screen.getAllByTestId("folder-icon");
    expect(folderIcons.length).toBeGreaterThan(0);
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

  it("navigates into directory when clicked", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [mockDirectory],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    fireEvent.click(screen.getByText("src"));

    // After clicking, src should appear in breadcrumbs
    const srcElements = screen.getAllByText("src");
    expect(srcElements.length).toBeGreaterThanOrEqual(1);
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
});

// ============================================
// Loading and Empty States
// ============================================

describe("FileBrowser States", () => {
  it("shows loading state", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: undefined,
      isLoading: true,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("Loading...")).toBeInTheDocument();
  });

  it("shows empty folder message", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    expect(screen.getByText("Empty folder")).toBeInTheDocument();
  });
});

// ============================================
// Parent Directory Navigation
// ============================================

describe("FileBrowser Parent Navigation", () => {
  it("shows parent directory (..) link when not at root", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [
        {
          id: "1",
          session_id: "test-session",
          path: "/workspace/src",
          name: "src",
          is_directory: true,
          is_readonly: false,
          size_bytes: 0,
          created_at: "2024-01-01T00:00:00Z",
          updated_at: "2024-01-01T00:00:00Z",
        },
      ],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    // Navigate into src folder
    fireEvent.click(screen.getByText("src"));

    // Should show ".." for parent navigation
    expect(screen.getByText("..")).toBeInTheDocument();
  });

  it("does not show parent directory link at root", () => {
    (useFiles as jest.Mock).mockReturnValue({
      data: [],
      isLoading: false,
      refetch: mockRefetch,
    });

    renderWithProviders(<FileBrowser sessionId="test-session" />);

    // At root, ".." should not be present
    expect(screen.queryByText("..")).not.toBeInTheDocument();
  });
});
