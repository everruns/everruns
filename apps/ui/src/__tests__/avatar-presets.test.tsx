import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { searchAvatarPresets } from "@/lib/avatar-presets";
import { AvatarPresetDialog } from "@/components/agents/avatar-preset-dialog";
import type { AvatarPreset } from "@/lib/api/agents";
import type { Agent } from "@/lib/api/types";

const presets: AvatarPreset[] = [
  {
    id: "familiars-patch",
    name: "Patch",
    family: "Familiars",
    role: "Coding",
    description: "Terracotta fox with navy glasses.",
    keywords: ["developer", "programmer", "engineer", "geometric"],
  },
  {
    id: "watchers-relay",
    name: "Relay",
    family: "Watchers",
    role: "Support",
    description: "Ivory arch with gold headset.",
    keywords: ["help", "customer service"],
  },
  {
    id: "totems-focus",
    name: "Focus",
    family: "Totems",
    role: "Reviewing",
    description: "Graphite ring framing a gold square.",
    keywords: ["reviewer", "code review", "audit", "quality", "ceramic"],
  },
];
const mockCatalog = {
  data: presets,
  isLoading: false,
  error: null as Error | null,
  refetch: jest.fn(),
};
const mockCurrent = { data: { preset_id: "watchers-relay" }, error: null };
jest.mock("@/hooks", () => ({
  useAvatarPresets: () => ({ catalog: mockCatalog, current: mockCurrent }),
}));
const agent = { id: "agent_1" } as Agent;
const props = {
  agent,
  open: true,
  disabled: false,
  saving: false,
  error: null,
  onClose: jest.fn(),
  onSave: jest.fn().mockResolvedValue(undefined),
};

beforeEach(() => {
  jest.clearAllMocks();
  mockCatalog.error = null;
});

describe("preset search", () => {
  it("shows every preset for empty or whitespace queries", () => {
    expect(searchAvatarPresets(presets, "")).toEqual(presets);
    expect(searchAvatarPresets(presets, " \t ")).toEqual(presets);
  });
  it.each([" Patch ", "PROGRAMMER", "terracotta", "coding fox", "Familiars navy engineer"])(
    "matches names, synonyms and cross-field terms: %s",
    (query) => {
      expect(searchAvatarPresets(presets, query).map((p) => p.id)).toEqual(["familiars-patch"]);
    },
  );
  it("matches multiword synonyms and family filters", () => {
    expect(searchAvatarPresets(presets, "customer service")[0].name).toBe("Relay");
    expect(searchAvatarPresets(presets, "code review audit quality")[0].name).toBe("Focus");
    expect(searchAvatarPresets(presets, "gold", "Totems")[0].name).toBe("Focus");
    expect(searchAvatarPresets(presets, "fox", "Totems")).toEqual([]);
    expect(searchAvatarPresets(presets, "missing")).toEqual([]);
  });
});

describe("preset picker", () => {
  it("shows current state, searches, selects and saves by stable ID", async () => {
    render(<AvatarPresetDialog {...props} />);
    expect(screen.getByRole("button", { name: /Relay, Watchers/ })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    fireEvent.change(screen.getByRole("textbox", { name: "Search avatars" }), {
      target: { value: "navy developer" },
    });
    expect(screen.queryByRole("button", { name: /Relay, Watchers/ })).toBeNull();
    const patch = screen.getByRole("button", { name: /Patch, Familiars/ });
    expect(patch).toHaveAccessibleDescription("Terracotta fox with navy glasses.");
    fireEvent.click(patch);
    expect(screen.getByAltText("Patch square preview")).toHaveAttribute(
      "src",
      "/api/v1/avatar-presets/familiars-patch/square-256.png",
    );
    expect(screen.getByAltText("Patch circular preview")).toHaveAttribute(
      "src",
      "/api/v1/avatar-presets/familiars-patch/circle-256.png",
    );
    fireEvent.click(screen.getByRole("button", { name: "Use avatar" }));
    await waitFor(() => expect(props.onSave).toHaveBeenCalledWith("familiars-patch"));
  });
  it("offers an empty state and clear search", () => {
    render(<AvatarPresetDialog {...props} />);
    fireEvent.change(screen.getByRole("textbox", { name: "Search avatars" }), {
      target: { value: "missing" },
    });
    expect(screen.getByText("No avatars match your search.")).toBeInTheDocument();
    fireEvent.click(screen.getByRole("button", { name: "Clear search" }));
    expect(screen.getByRole("button", { name: /Patch, Familiars/ })).toBeInTheDocument();
  });
  it("enforces busy/read-only and displays actionable save errors", () => {
    const { rerender } = render(<AvatarPresetDialog {...props} disabled saving />);
    expect(screen.getByRole("button", { name: "Saving avatar…" })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Patch, Familiars/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: "Cancel" })).toBeDisabled();
    rerender(<AvatarPresetDialog {...props} disabled error="Could not save avatar. Try again." />);
    expect(screen.getByRole("alert")).toHaveTextContent("Try again");
    expect(screen.getByRole("button", { name: "Use avatar" })).toBeDisabled();
  });
  it("retries catalog failures", () => {
    mockCatalog.error = new Error("offline");
    render(<AvatarPresetDialog {...props} />);
    fireEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(mockCatalog.refetch).toHaveBeenCalled();
  });
  it("keeps failed saves available for retry and does not submit a parent form", async () => {
    const onSubmit = jest.fn();
    const onSave = jest.fn().mockRejectedValue(new Error("offline"));
    render(
      <form onSubmit={onSubmit}>
        <AvatarPresetDialog {...props} onSave={onSave} error="Offline. Retry." />
      </form>,
    );
    fireEvent.click(screen.getByRole("button", { name: "Use avatar" }));
    await waitFor(() => expect(onSave).toHaveBeenCalledWith("watchers-relay"));
    expect(onSubmit).not.toHaveBeenCalled();
    expect(screen.getByRole("dialog")).toBeInTheDocument();
  });
});
