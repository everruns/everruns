import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Suspense } from "react";
import MemoryDetailPage from "@/app/(main)/memory/[memoryId]/page";
import type { Memory } from "@/lib/api/types";

const mockUseMemory = jest.fn();
const mockUseUpdateMemory = jest.fn();
const mockUseSyncMemory = jest.fn();
const mockUseArchiveMemory = jest.fn();
const mockUseUserConnections = jest.fn();

jest.mock("@/hooks", () => ({
  useMemory: (...args: unknown[]) => mockUseMemory(...args),
  useUpdateMemory: () => mockUseUpdateMemory(),
  useSyncMemory: () => mockUseSyncMemory(),
  useArchiveMemory: () => mockUseArchiveMemory(),
  useUserConnections: () => mockUseUserConnections(),
  usePageTitle: () => undefined,
}));

jest.mock("@/hooks/use-user-connections", () => ({
  useUserConnections: () => mockUseUserConnections(),
}));

const memory: Memory = {
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
};

async function renderWithSuspense(memoryId = memory.id) {
  const paramsPromise = Promise.resolve({ memoryId });

  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <MemoryDetailPage params={paramsPromise} />
      </Suspense>,
    );
    await paramsPromise;
  });
}

describe("MemoryDetailPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseMemory.mockReturnValue({ data: memory, isLoading: false, error: null });
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

  it("renders memory metadata", async () => {
    await renderWithSuspense();

    expect(screen.getByRole("heading", { level: 1, name: "Research" })).toBeInTheDocument();
    expect(screen.getByText("Shared research files")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Memory/i })).toHaveAttribute("href", "/memory");
  });

  it("updates memory metadata", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateMemory.mockReturnValue({ mutateAsync, isPending: false });
    await renderWithSuspense();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "Research Hub" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save Changes" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        memoryId: memory.id,
        data: { name: "Research Hub", description: "Shared research files" },
      }),
    );
  });

  it("archives the memory after confirmation", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseArchiveMemory.mockReturnValue({ mutateAsync, isPending: false });
    await renderWithSuspense();

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledWith(memory.id));
  });

  it("treats deleted memory as read-only", async () => {
    mockUseMemory.mockReturnValue({
      data: {
        ...memory,
        status: "deleted",
        deleted_at: "2026-05-06T03:37:51.552849Z",
      },
      isLoading: false,
      error: null,
    });

    await renderWithSuspense();

    expect(screen.getByRole("button", { name: "Edit" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Archive" })).not.toBeInTheDocument();
  });

  it("renders not found state", async () => {
    mockUseMemory.mockReturnValue({ data: undefined, isLoading: false, error: new Error("404") });

    await renderWithSuspense("mem_missing");

    expect(screen.getByText("Memory not found")).toBeInTheDocument();
    expect(screen.getByText("mem_missing")).toBeInTheDocument();
  });
});
