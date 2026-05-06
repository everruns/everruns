import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { Suspense } from "react";
import VolumeDetailPage from "@/app/(main)/volumes/[volumeId]/page";
import type { Volume } from "@/lib/api/types";

const mockUseVolume = jest.fn();
const mockUseUpdateVolume = jest.fn();
const mockUseArchiveVolume = jest.fn();

jest.mock("@/hooks", () => ({
  useVolume: (...args: unknown[]) => mockUseVolume(...args),
  useUpdateVolume: () => mockUseUpdateVolume(),
  useArchiveVolume: () => mockUseArchiveVolume(),
  usePageTitle: () => undefined,
}));

const volume: Volume = {
  id: "vol_019dfb261a407c6085dcdd602402c3f7",
  name: "Research",
  description: "Shared research files",
  status: "active",
  created_at: "2026-05-06T02:37:51.552849Z",
  updated_at: "2026-05-06T02:37:51.552849Z",
  archived_at: null,
  deleted_at: null,
};

async function renderWithSuspense(volumeId = volume.id) {
  const paramsPromise = Promise.resolve({ volumeId });

  await act(async () => {
    render(
      <Suspense fallback={<div>Loading...</div>}>
        <VolumeDetailPage params={paramsPromise} />
      </Suspense>,
    );
    await paramsPromise;
  });
}

describe("VolumeDetailPage", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseVolume.mockReturnValue({ data: volume, isLoading: false, error: null });
    mockUseUpdateVolume.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
    mockUseArchiveVolume.mockReturnValue({
      mutateAsync: jest.fn().mockResolvedValue({}),
      isPending: false,
    });
  });

  it("renders volume metadata", async () => {
    await renderWithSuspense();

    expect(screen.getByRole("heading", { level: 1, name: "Research" })).toBeInTheDocument();
    expect(screen.getByText("Shared research files")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: /Volumes/i })).toHaveAttribute("href", "/volumes");
  });

  it("updates volume metadata", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseUpdateVolume.mockReturnValue({ mutateAsync, isPending: false });
    await renderWithSuspense();

    fireEvent.click(screen.getByRole("button", { name: "Edit" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.change(within(dialog).getByLabelText("Name"), { target: { value: "Research Hub" } });
    fireEvent.click(within(dialog).getByRole("button", { name: "Save Changes" }));

    await waitFor(() =>
      expect(mutateAsync).toHaveBeenCalledWith({
        volumeId: volume.id,
        data: { name: "Research Hub", description: "Shared research files" },
      }),
    );
  });

  it("archives the volume after confirmation", async () => {
    const mutateAsync = jest.fn().mockResolvedValue({});
    mockUseArchiveVolume.mockReturnValue({ mutateAsync, isPending: false });
    await renderWithSuspense();

    fireEvent.click(screen.getByRole("button", { name: "Archive" }));
    const dialog = screen.getByRole("dialog");
    fireEvent.click(within(dialog).getByRole("button", { name: "Archive" }));

    await waitFor(() => expect(mutateAsync).toHaveBeenCalledWith(volume.id));
  });

  it("treats deleted volumes as read-only", async () => {
    mockUseVolume.mockReturnValue({
      data: {
        ...volume,
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
    mockUseVolume.mockReturnValue({ data: undefined, isLoading: false, error: new Error("404") });

    await renderWithSuspense("vol_missing");

    expect(screen.getByText("Volume not found")).toBeInTheDocument();
    expect(screen.getByText("vol_missing")).toBeInTheDocument();
  });
});
