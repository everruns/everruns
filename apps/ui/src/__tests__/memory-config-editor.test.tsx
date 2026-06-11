import { render, screen, fireEvent } from "@testing-library/react";
import {
  MemoryConfigEditor,
  pathsOverlap,
  validateMountPath,
} from "@/components/agents/memory-config-editor";

const VOLUME_A = "mem_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const VOLUME_B = "mem_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

jest.mock("@/hooks/use-memory", () => ({
  useMemories: jest.fn(),
}));

const { useMemories } = jest.requireMock("@/hooks/use-memory") as {
  useMemories: jest.Mock;
};

beforeEach(() => {
  useMemories.mockReturnValue({
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

  it("rejects traversal, empty segments, and trailing slash", () => {
    expect(validateMountPath("/workspace/..")).not.toBeNull();
    expect(validateMountPath("/workspace//x")).not.toBeNull();
    expect(validateMountPath("/workspace/")).not.toBeNull();
  });

  it("treats empty paths as soft (no error message yet)", () => {
    // Empty paths return null so freshly-added rows do not start in an error
    // state; the server still rejects empty paths on save.
    expect(validateMountPath("")).toBeNull();
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

  it("compares by path segments (handles non-ASCII without divergence)", () => {
    // Multi-byte segments must not be falsely flagged as overlapping when the
    // longer path begins with a different segment that shares a UTF-8 prefix.
    expect(pathsOverlap("/workspace/é", "/workspace/échec")).toBe(false);
    expect(pathsOverlap("/workspace/é", "/workspace/é/sub")).toBe(true);
  });
});

describe("MemoryConfigEditor", () => {
  it("renders empty state when no mounts and no memory", () => {
    useMemories.mockReturnValue({ data: [], isLoading: false, error: null });
    render(<MemoryConfigEditor config={{}} onChange={jest.fn()} />);
    expect(screen.getByText(/no active memory/i)).toBeInTheDocument();
  });

  it("renders existing mount rows", () => {
    render(
      <MemoryConfigEditor
        config={{
          mounts: [{ memory: VOLUME_A, path: "/workspace/research", mode: "readonly" }],
        }}
        onChange={jest.fn()}
      />,
    );
    expect(screen.getByDisplayValue("/workspace/research")).toBeInTheDocument();
    expect(screen.getAllByTestId("memory-mount-row")).toHaveLength(1);
  });

  it("treats null config as empty mounts", () => {
    render(<MemoryConfigEditor config={null} onChange={jest.fn()} />);
    expect(screen.queryByTestId("memory-mount-row")).not.toBeInTheDocument();
  });

  it("calls onChange when adding a mount row with empty defaults", () => {
    const onChange = jest.fn();
    render(<MemoryConfigEditor config={{}} onChange={onChange} />);
    fireEvent.click(screen.getByRole("button", { name: /add mount/i }));
    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({
        mounts: [{ memory: "", path: "", mode: "readonly" }],
      }),
    );
  });

  it("shows error for invalid mount path", () => {
    render(
      <MemoryConfigEditor
        config={{
          mounts: [{ memory: VOLUME_A, path: "/etc/passwd", mode: "readonly" }],
        }}
        onChange={jest.fn()}
      />,
    );
    expect(screen.getByText(/must be \/workspace/i)).toBeInTheDocument();
  });

  it("shows overlap error on both rows", () => {
    render(
      <MemoryConfigEditor
        config={{
          mounts: [
            { memory: VOLUME_A, path: "/workspace/data" },
            { memory: VOLUME_B, path: "/workspace/data/inner" },
          ],
        }}
        onChange={jest.fn()}
      />,
    );
    // Both the shorter and longer paths should be flagged so the conflict is
    // visible regardless of row order.
    expect(screen.getAllByText(/overlaps with/i)).toHaveLength(2);
  });

  it("does not flag unknown memory while the memory list is loading", () => {
    useMemories.mockReturnValue({ data: undefined, isLoading: true, error: null });
    render(
      <MemoryConfigEditor
        config={{ mounts: [{ memory: VOLUME_A, path: "/workspace/x" }] }}
        onChange={jest.fn()}
      />,
    );
    expect(screen.queryByText(/unavailable or archived/i)).not.toBeInTheDocument();
  });

  it("flags memory IDs that are not in the active list", () => {
    render(
      <MemoryConfigEditor
        config={{
          mounts: [
            {
              memory: "mem_99999999999999999999999999999999",
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
      <MemoryConfigEditor
        config={{
          mounts: [
            { memory: VOLUME_A, path: "/workspace/a", mode: "readonly" },
            { memory: VOLUME_B, path: "/workspace/b", mode: "readwrite" },
          ],
        }}
        onChange={onChange}
      />,
    );
    const removeButtons = screen.getAllByRole("button", { name: /remove mount/i });
    fireEvent.click(removeButtons[0]);
    expect(onChange).toHaveBeenCalledWith({
      mounts: [{ memory: VOLUME_B, path: "/workspace/b", mode: "readwrite" }],
    });
  });

  it("clears mounts to empty config when last row removed", () => {
    const onChange = jest.fn();
    render(
      <MemoryConfigEditor
        config={{
          mounts: [{ memory: VOLUME_A, path: "/workspace/x", mode: "readonly" }],
        }}
        onChange={onChange}
      />,
    );
    fireEvent.click(screen.getByRole("button", { name: /remove mount/i }));
    expect(onChange).toHaveBeenCalledWith({});
  });
});
