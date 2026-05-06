import { render, screen, fireEvent } from "@testing-library/react";
import {
  WorkspaceVolumesConfigEditor,
  pathsOverlap,
  validateMountPath,
} from "@/components/agents/workspace-volumes-config-editor";

const VOLUME_A = "vol_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VOLUME_B = "vol_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

jest.mock("@/hooks/use-volumes", () => ({
  useVolumes: jest.fn(),
}));

const { useVolumes } = jest.requireMock("@/hooks/use-volumes") as {
  useVolumes: jest.Mock;
};

beforeEach(() => {
  useVolumes.mockReturnValue({
    data: [
      { id: VOLUME_A, name: "Research", status: "active" },
      { id: VOLUME_B, name: "Reports", status: "active" },
    ],
    isLoading: false,
    error: null,
  });
});

describe("validateMountPath", () => {
  it("accepts /workspace and /workspace/...", () => {
    expect(validateMountPath("/workspace")).toBeNull();
    expect(validateMountPath("/workspace/research")).toBeNull();
    expect(validateMountPath("/workspace/a/b")).toBeNull();
  });

  it("rejects paths outside /workspace", () => {
    expect(validateMountPath("/etc")).not.toBeNull();
    expect(validateMountPath("/workspacefoo")).not.toBeNull();
  });

  it("rejects empty, traversal, and trailing slash", () => {
    expect(validateMountPath("")).not.toBeNull();
    expect(validateMountPath("/workspace/..")).not.toBeNull();
    expect(validateMountPath("/workspace//x")).not.toBeNull();
    expect(validateMountPath("/workspace/")).not.toBeNull();
  });
});

describe("pathsOverlap", () => {
  it("detects equal paths", () => {
    expect(pathsOverlap("/workspace/a", "/workspace/a")).toBe(true);
  });

  it("detects prefix overlap on path boundary", () => {
    expect(pathsOverlap("/workspace/a", "/workspace/a/b")).toBe(true);
  });

  it("does not flag paths sharing prefix without boundary", () => {
    expect(pathsOverlap("/workspace/abc", "/workspace/abcdef")).toBe(false);
  });

  it("does not flag disjoint paths", () => {
    expect(pathsOverlap("/workspace/a", "/workspace/b")).toBe(false);
  });
});

describe("WorkspaceVolumesConfigEditor", () => {
  it("renders empty state when no mounts and no volumes", () => {
    useVolumes.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<WorkspaceVolumesConfigEditor config={{}} onChange={jest.fn()} />);
    expect(screen.getByText(/no active volumes/i)).toBeInTheDocument();
  });

  it("renders existing mount rows", () => {
    render(
      <WorkspaceVolumesConfigEditor
        config={{
          mounts: [{ volume: VOLUME_A, path: "/workspace/research", mode: "readonly" }],
        }}
        onChange={jest.fn()}
      />,
    );
    expect(screen.getByDisplayValue("/workspace/research")).toBeInTheDocument();
    expect(screen.getAllByTestId("volume-mount-row")).toHaveLength(1);
  });

  it("calls onChange when adding a mount row", () => {
    const onChange = jest.fn();
    render(<WorkspaceVolumesConfigEditor config={{}} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /add mount/i }));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        mounts: [expect.objectContaining({ path: "/workspace/", mode: "readonly" })],
      }),
    );
  });

  it("shows error for invalid mount path", () => {
    render(
      <WorkspaceVolumesConfigEditor
        config={{
          mounts: [{ volume: VOLUME_A, path: "/etc/passwd", mode: "readonly" }],
        }}
        onChange={jest.fn()}
      />,
    );
    expect(screen.getByText(/must be \/workspace/i)).toBeInTheDocument();
  });

  it("shows error for duplicate / overlapping mounts", () => {
    render(
      <WorkspaceVolumesConfigEditor
        config={{
          mounts: [
            { volume: VOLUME_A, path: "/workspace/data" },
            { volume: VOLUME_B, path: "/workspace/data/inner" },
          ],
        }}
        onChange={jest.fn()}
      />,
    );
    expect(screen.getByText(/overlaps with/i)).toBeInTheDocument();
  });

  it("flags volume IDs that are not in the active list", () => {
    render(
      <WorkspaceVolumesConfigEditor
        config={{
          mounts: [
            {
              volume: "vol_99999999999999999999999999999999",
              path: "/workspace/x",
            },
          ],
        }}
        onChange={jest.fn()}
      />,
    );
    expect(screen.getByText(/unavailable or archived/i)).toBeInTheDocument();
  });

  it("removes a mount row on trash click", () => {
    const onChange = jest.fn();
    render(
      <WorkspaceVolumesConfigEditor
        config={{
          mounts: [
            { volume: VOLUME_A, path: "/workspace/a", mode: "readonly" },
            { volume: VOLUME_B, path: "/workspace/b", mode: "readwrite" },
          ],
        }}
        onChange={onChange}
      />,
    );
    const removeButtons = screen.getAllByRole("button", { name: /remove mount/i });
    fireEvent.click(removeButtons[0]);
    expect(onChange).toHaveBeenCalledWith({
      mounts: [{ volume: VOLUME_B, path: "/workspace/b", mode: "readwrite" }],
    });
  });

  it("clears mounts to empty config when last row removed", () => {
    const onChange = jest.fn();
    render(
      <WorkspaceVolumesConfigEditor
        config={{
          mounts: [{ volume: VOLUME_A, path: "/workspace/x", mode: "readonly" }],
        }}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /remove mount/i }));
    expect(onChange).toHaveBeenCalledWith({});
  });
});
