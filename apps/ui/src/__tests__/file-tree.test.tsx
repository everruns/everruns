import { render, screen, fireEvent } from "@testing-library/react";
import {
  FileTree,
  FileTreeFolder,
  FileTreeFile,
  FileTreeActions,
  FileTreeName,
} from "@/components/ai-elements/file-tree";

// Mock lucide-react icons with className forwarding
jest.mock("lucide-react", () => ({
  ChevronRightIcon: ({ className }: { className?: string }) => (
    <span data-testid="chevron-icon" className={className} />
  ),
  FileIcon: () => <span data-testid="file-icon" />,
  FolderIcon: () => <span data-testid="folder-icon" />,
  FolderOpenIcon: () => <span data-testid="folder-open-icon" />,
}));

describe("FileTree", () => {
  describe("selection", () => {
    it("calls onSelect when file is clicked", () => {
      const onSelect = jest.fn();
      render(
        <FileTree onSelect={onSelect}>
          <FileTreeFile path="/test.txt" name="test.txt" />
        </FileTree>,
      );

      fireEvent.click(screen.getByText("test.txt"));
      expect(onSelect).toHaveBeenCalledWith("/test.txt");
    });

    it("highlights selected file", () => {
      render(
        <FileTree selectedPath="/test.txt">
          <FileTreeFile path="/test.txt" name="test.txt" />
          <FileTreeFile path="/other.txt" name="other.txt" />
        </FileTree>,
      );

      const selectedFile = screen.getByText("test.txt").closest("[role='treeitem']");
      expect(selectedFile).toHaveClass("bg-accent/20");
    });

    it("does not highlight unselected files", () => {
      render(
        <FileTree selectedPath="/test.txt">
          <FileTreeFile path="/other.txt" name="other.txt" />
        </FileTree>,
      );

      const unselectedFile = screen.getByText("other.txt").closest("[role='treeitem']");
      expect(unselectedFile).not.toHaveClass("bg-accent/20");
    });
  });

  describe("keyboard navigation", () => {
    it("selects file on Enter key", () => {
      const onSelect = jest.fn();
      render(
        <FileTree onSelect={onSelect}>
          <FileTreeFile path="/test.txt" name="test.txt" />
        </FileTree>,
      );

      const file = screen.getByRole("treeitem");
      fireEvent.keyDown(file, { key: "Enter" });
      expect(onSelect).toHaveBeenCalledWith("/test.txt");
    });

    it("selects file on Space key", () => {
      const onSelect = jest.fn();
      render(
        <FileTree onSelect={onSelect}>
          <FileTreeFile path="/test.txt" name="test.txt" />
        </FileTree>,
      );

      const file = screen.getByRole("treeitem");
      fireEvent.keyDown(file, { key: " " });
      expect(onSelect).toHaveBeenCalledWith("/test.txt");
    });
  });
});

describe("FileTreeFolder", () => {
  describe("expansion", () => {
    it("is collapsed by default", () => {
      render(
        <FileTree>
          <FileTreeFolder path="/src" name="src">
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      // The trigger button should indicate closed state
      const trigger = screen.getByRole("button", { expanded: false });
      expect(trigger).toHaveAttribute("aria-expanded", "false");
    });

    it("expands when in defaultExpanded set", () => {
      render(
        <FileTree defaultExpanded={new Set(["/src"])}>
          <FileTreeFolder path="/src" name="src">
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      expect(screen.getByText("index.ts")).toBeVisible();
    });

    it("expands when in controlled expanded set", () => {
      render(
        <FileTree expanded={new Set(["/src"])}>
          <FileTreeFolder path="/src" name="src">
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      expect(screen.getByText("index.ts")).toBeVisible();
    });

    it("toggles expansion on click", () => {
      const onExpandedChange = jest.fn();
      render(
        <FileTree expanded={new Set()} onExpandedChange={onExpandedChange}>
          <FileTreeFolder path="/src" name="src">
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      fireEvent.click(screen.getByText("src"));

      expect(onExpandedChange).toHaveBeenCalled();
      const newExpanded = onExpandedChange.mock.calls[0][0];
      expect(newExpanded.has("/src")).toBe(true);
    });
  });

  describe("icons", () => {
    it("shows closed folder icon when collapsed", () => {
      render(
        <FileTree>
          <FileTreeFolder path="/src" name="src">
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      expect(screen.getByTestId("folder-icon")).toBeInTheDocument();
    });

    it("shows open folder icon when expanded", () => {
      render(
        <FileTree expanded={new Set(["/src"])}>
          <FileTreeFolder path="/src" name="src">
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      expect(screen.getByTestId("folder-open-icon")).toBeInTheDocument();
    });

    it("rotates chevron when expanded", () => {
      render(
        <FileTree expanded={new Set(["/src"])}>
          <FileTreeFolder path="/src" name="src">
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      // The ChevronRightIcon receives rotate-90 class when expanded
      const chevron = screen.getByTestId("chevron-icon");
      expect(chevron).toHaveClass("rotate-90");
    });
  });

  describe("actions prop", () => {
    it("renders actions inline in the folder row", () => {
      render(
        <FileTree expanded={new Set(["/src"])}>
          <FileTreeFolder
            path="/src"
            name="src"
            actions={
              <FileTreeActions>
                <span data-testid="inline-action">Add</span>
              </FileTreeActions>
            }
          >
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      const action = screen.getByTestId("inline-action");
      expect(action).toBeInTheDocument();
      // Action and folder name share the same row container (not in collapsible content)
      const folderName = screen.getByText("src");
      const row = folderName.closest("button")!.parentElement!;
      expect(row).toContainElement(action);
    });

    it("keeps actions outside the trigger button (no nested buttons)", () => {
      render(
        <FileTree expanded={new Set(["/src"])}>
          <FileTreeFolder
            path="/src"
            name="src"
            actions={
              <FileTreeActions>
                <span data-testid="inline-action">Add</span>
              </FileTreeActions>
            }
          >
            <FileTreeFile path="/src/index.ts" name="index.ts" />
          </FileTreeFolder>
        </FileTree>,
      );

      // Actions should be a sibling of the trigger, not nested inside it
      const trigger = screen.getByRole("button", { expanded: true });
      const action = screen.getByTestId("inline-action");
      expect(trigger).not.toContainElement(action);
    });
  });
});

describe("FileTreeFile", () => {
  describe("rendering", () => {
    it("renders custom icon", () => {
      const CustomIcon = () => <span data-testid="custom-icon" />;
      render(
        <FileTree>
          <FileTreeFile path="/test.txt" name="test.txt" icon={<CustomIcon />} />
        </FileTree>,
      );

      expect(screen.getByTestId("custom-icon")).toBeInTheDocument();
    });
  });
});

describe("FileTreeActions", () => {
  it("stops click propagation", () => {
    const onSelect = jest.fn();
    const onAction = jest.fn();

    render(
      <FileTree onSelect={onSelect}>
        <FileTreeFile path="/test.txt" name="test.txt">
          <FileTreeName>test.txt</FileTreeName>
          <FileTreeActions>
            <button onClick={onAction}>Delete</button>
          </FileTreeActions>
        </FileTreeFile>
      </FileTree>,
    );

    fireEvent.click(screen.getByText("Delete"));

    expect(onAction).toHaveBeenCalled();
    expect(onSelect).not.toHaveBeenCalled();
  });
});
