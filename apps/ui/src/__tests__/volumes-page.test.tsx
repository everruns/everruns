import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import VolumesPage from "@/app/(main)/volumes/page";
import type { Volume } from "@/lib/api/types";

const mockUseVolumes = jest.fn();
const mockUseCreateVolume = jest.fn();
const mockUseUpdateVolume = jest.fn();
const mockUseArchiveVolume = jest.fn();

jest.mock("@/hooks", () => ({
  useVolumes: (...args: unknown[]) => mockUseVolumes(...args),
  useCreateVolume: () => mockUseCreateVolume(),
  useUpdateVolume: () => mockUseUpdateVolume(),
  useArchiveVolume: () => mockUseArchiveVolume(),
  usePageTitle: () => undefined,
}));

const volumes: Volume[] = [
  {
    id: "vol_019dfb261a407c6085dcdd602402c3f7",
    name: "Research",
    description: "Shared research files",
    status: "active",
    created_at: "2026-05-06T02:37:51.552849Z",
    updated_at: "2026-05-06T02:37:51.552849Z",
    archived_at: null,
    deleted_at: null,
  },
];

describe("VolumesPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseVolumes.mockReturnValue({ data: volumes, isLoading: false, error: null });
    mockUseCreateVolume.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseUpdateVolume.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseArchiveVolume.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
  });

  it("renders workspace volumes", () => {
    render(<VolumesPage />);

    expect(screen.getByRole("heading", { level: 1, name: "Volumes" })).toBeInTheDocument();
    expect(screen.getByText("Research")).toBeInTheDocument();
    expect(screen.getByText("Shared research files")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Open/i })).toHaveAttribute(
      "href",
      "/volumes/vol_019dfb261a407c6085dcdd602402c3f7",
    );
  });

  it("passes search text into the volumes query", () => {
    render(<VolumesPage />);

    fireEvent.change(screen.getByLabelText("Search volumes"), { target: { value: "research" } });

    expect(mockUseVolumes).toHaveBeenLastCalledWith({
      includeArchived: false,
      search: "research",
    });
  });

  it("creates a volume from the dialog", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseCreateVolume.mockReturnValue({ mutateAsync, isPending: false });
    render(<VolumesPage />);

    fireEvent.click(screen.getByRole("button", { name: "New Volume" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "Runbooks" } });
    fireEvent.change(within(dialog).getByLabelText("Description"), {
      target: { value: "Operations notes" },
    });
    fireEvent.click(within(dialog).getByRole("button", { name: "Create Volume" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        name: "Runbooks",
        description: "Operations notes",
      }),
    );
  });

  it("updates a volume and clears blank descriptions", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateVolume.mockReturnValue({ mutateAsync, isPending: false });
    render(<VolumesPage />);

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "Research Hub" } });
    fireEvent.change(within(dialog).getByLabelText("Description"), { target: { value: "" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save Changes" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        volumeId: "vol_019dfb261a407c6085dcdd602402c3f7",
        data: { name: "Research Hub", description: null },
      }),
    );
  });

  it("archives a volume after confirmation", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseArchiveVolume.mockReturnValue({ mutateAsync, isPending: false });
    render(<VolumesPage />);

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    const dialog = screen.getByRole("dialog");
    expect(within(dialog).getByText("Archive Research?")).toBeInTheDocument();
    fireEvent.click(within(dialog).getByRole("button", { name: "Archive" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith("vol_019dfb261a407c6085dcdd602402c3f7"),
    );
  });

  it("treats deleted volumes as read-only", () => {
    mockUseVolumes.mockReturnValue({
      data: [
        {
          ...volumes[0],
          status: "deleted",
          deleted_at: "2026-05-06T03:37:51.552849Z",
        },
      ],
      isLoading: false,
      error: null,
    });

    render(<VolumesPage />);

    expect(screen.getByRole("button", { name: "Edit" })).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Archive" })).not.toBeInTheDocument();
  });

  it("renders the empty state", () => {
    mockUseVolumes.mockReturnValue({ data: [], isLoading: false, error: null });

    render(<VolumesPage />);

    expect(screen.getByText("No volumes")).toBeInTheDocument();
  });
});
