import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import * as mockReact from "react";
import MemoryPage from "@/app/(main)/memory/page";
import type { Memory } from "@/lib/api/types";

const mockUseMemories = jest.fn();
const mockUseCreateMemory = jest.fn();
const mockUseUpdateMemory = jest.fn();
const mockUseSyncMemory = jest.fn();
const mockUseArchiveMemory = jest.fn();
const mockUseUserConnections = jest.fn();

jest.mock("@/hooks", () => ({
  useMemories: (...args: unknown[]) => mockUseMemories(...args),
  useCreateMemory: () => mockUseCreateMemory(),
  useUpdateMemory: () => mockUseUpdateMemory(),
  useSyncMemory: () => mockUseSyncMemory(),
  useArchiveMemory: () => mockUseArchiveMemory(),
  useUserConnections: () => mockUseUserConnections(),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useUserConnections: () => mockUseUserConnections(),
}));

jest.mock("@/components/ui/select", () => {
  function collectOptions(node: mockReact.ReactNode): mockReact.ReactNode[] {
    const options: mockReact.ReactNode[] = [];
    mockReact.Children.forEach(node, (child: mockReact.ReactNode) => {
      if (!mockReact.isValidElement(child)) return;
      if ((child.type as { isSelectItem?: boolean }).isSelectItem) {
        const props = child.props as {
          value: string;
          children: mockReact.ReactNode;
        };
        options.push(
          <option key={props.value} value={props.value}>
            {props.children}
          </option>,
        );
        return;
      }
      options.push(...collectOptions((child.props as { children?: mockReact.ReactNode }).children));
    });
    return options;
  }

  function Select({
    value,
    onValueChange,
    children,
  }: {
    value: string;
    onValueChange: (value: string) => void;
    children: mockReact.ReactNode;
  }) {
    return (
      <select
        aria-label="Source"
        value={value}
        onChange={(event) => onValueChange(event.target.value)}
      >
        {collectOptions(children)}
      </select>
    );
  }

  function SelectItem(_: { value: string; children: mockReact.ReactNode }) {
    return null;
  }
  SelectItem.isSelectItem = true;

  return {
    Select,
    SelectContent: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectItem,
    SelectTrigger: ({ children }: { children: mockReact.ReactNode }) => <>{children}</>,
    SelectValue: () => null,
  };
});

const memory: Memory[] = [
  {
    id: "mem_019dfb261a407c6085dcdd602402c3f7",
    name: "Research",
    description: "Shared research files",
    source_type: "manual",
    source: { provider: "manual" },
    is_readonly: false,
    sync_status: "idle",
    last_synced_at: null,
    last_sync_error: null,
    status: "active",
    created_at: "2026-05-06T02:37:51.552849Z",
    updated_at: "2026-05-06T02:37:51.552849Z",
    archived_at: null,
    deleted_at: null,
  },
];

describe("MemoryPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseMemories.mockReturnValue({
      data: memory,
      isLoading: false,
      error: null,
    });
    mockUseCreateMemory.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseUpdateMemory.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseSyncMemory.mockReturnValue({
      mutate: jest.fn(),
      isPending: false,
    });
    mockUseUserConnections.mockReturnValue({
      data: [],
      isLoading: false,
    });
    mockUseArchiveMemory.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
  });

  it("renders memory", () => {
    render(<MemoryPage />);

    expect(screen.getByRole("heading", { level: 1, name: "Memory" })).toBeInTheDocument();
    expect(screen.getByText("Research")).toBeInTheDocument();
    expect(screen.getByText("Shared research files")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open/i })).toHaveAttribute(
      "href",
      "/memory/mem_019dfb261a407c6085dcdd602402c3f7",
    );
  });

  it("passes search text into the memory query", () => {
    render(<MemoryPage />);

    fireEvent.change(screen.getByLabelText("Search memory"), {
      target: { value: "research" },
    });

    expect(mockUseMemories).toHaveBeenLastCalledWith({
      includeArchived: false,
      search: "research",
    });
  });

  it("creates a memory from the dialog", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseCreateMemory.mockReturnValue({ mutateAsync, isPending: false });
    render(<MemoryPage />);

    fireEvent.click(screen.getByRole("button", { name: "New Memory" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "Runbooks" },
    });
    fireEvent.change(within(dialog).getByLabelText("Description"), {
      target: { value: "Operations notes" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create Memory" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Runbooks",
        description: "Operations notes",
      }),
    );
  });

  it("creates a github-backed read-only memory from the dialog", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseCreateMemory.mockReturnValue({ mutateAsync, isPending: false });
    render(<MemoryPage />);

    fireEvent.click(screen.getByRole("button", { name: "New Memory" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "Repo Docs" },
    });
    fireEvent.change(within(dialog).getByLabelText("Source"), {
      target: { value: "github" },
    });
    fireEvent.change(within(dialog).getByLabelText("Repository"), {
      target: { value: "everruns/everruns" },
    });
    fireEvent.change(within(dialog).getByLabelText("Branch"), {
      target: { value: "docs" },
    });
    fireEvent.change(within(dialog).getByLabelText("Root Folder"), {
      target: { value: "specs" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create Memory" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Repo Docs",
        source: {
          type: "github",
          repository: "everruns/everruns",
          branch: "docs",
          root_folder: "specs",
        },
      }),
    );
  });

  it("updates a memory and clears blank descriptions", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMemory.mockReturnValue({ mutateAsync, isPending: false });
    render(<MemoryPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "Research Hub" },
    });
    fireEvent.change(within(dialog).getByLabelText("Description"), {
      target: { value: "" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save Changes" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        memoryId: "mem_019dfb261a407c6085dcdd602402c3f7",
        data: { name: "Research Hub", description: null },
      }),
    );
  });

  it("does not resubmit unchanged source config on metadata-only edits", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMemory.mockReturnValue({ mutateAsync, isPending: false });
    mockUseMemories.mockReturnValue({
      data: [
        {
          ...memory[0],
          source_type: "github",
          source: {
            provider: "github",
            repository: "everruns/everruns",
            branch: "main",
            root_folder: "specs",
            sync_interval_secs: 900,
          },
          is_readonly: true,
          sync_status: "synced",
          last_synced_at: "2026-05-06T03:37:51.552849Z",
        },
      ],
      isLoading: false,
      error: null,
    });
    render(<MemoryPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), {
      target: { value: "Research Hub" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save Changes" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        memoryId: "mem_019dfb261a407c6085dcdd602402c3f7",
        data: { name: "Research Hub", description: "Shared research files" },
      }),
    );
  });

  it("archives a memory after confirmation", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseArchiveMemory.mockReturnValue({ mutateAsync, isPending: false });
    render(<MemoryPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Archive Research?")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Archive" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith("mem_019dfb261a407c6085dcdd602402c3f7"),
    );
  });

  it("treats deleted memory as read-only", () => {
    mockUseMemories.mockReturnValue({
      data: [
        {
          ...memory[0],
          status: "deleted",
          deleted_at: "2026-05-06T03:37:51.552849Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<MemoryPage />);

    expect(screen.getByRole("button", { name: "Edit" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Archive" })).not.toBeInTheDocument();
  });

  it("renders the empty state", () => {
    mockUseMemories.mockReturnValue({ data: [], isLoading: false, error: null });

    render(<MemoryPage />);

    expect(screen.getByText("No memory")).toBeInTheDocument();
  });
});
