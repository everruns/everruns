import { fireEvent, render, screen } from "@testing-library/react";
import type React from "react";
import { Boxes } from "lucide-react";
import type { SearchResult } from "@/hooks/use-global-search";
import { CommandPalette } from "@/components/command-palette";

const mockSetOpen = jest.fn();
const mockPush = jest.fn();
const mockOnSelect = jest.fn();

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: mockPush }),
  usePathname: () => "/dashboard",
}));

jest.mock("@base-ui/react/dialog", () => {
  const ReactRuntime = jest.requireActual<typeof import("react")>("react");
  return {
    Dialog: {
      Root: ({ open, children }: { open: boolean; children: React.ReactNode }) =>
        open ? ReactRuntime.createElement(ReactRuntime.Fragment, null, children) : null,
      Portal: ({ children }: { children: React.ReactNode }) =>
        ReactRuntime.createElement(ReactRuntime.Fragment, null, children),
      Backdrop: (props: React.HTMLAttributes<HTMLDivElement>) =>
        ReactRuntime.createElement("div", props),
      Title: (props: React.HTMLAttributes<HTMLHeadingElement>) =>
        ReactRuntime.createElement("h2", props),
      Description: (props: React.HTMLAttributes<HTMLParagraphElement>) =>
        ReactRuntime.createElement("p", props),
      Popup: (props: React.HTMLAttributes<HTMLDivElement>) =>
        ReactRuntime.createElement("div", props),
    },
  };
});

let mockOpen = true;
jest.mock("@/hooks/use-command-palette", () => ({
  useCommandPalette: () => ({
    open: mockOpen,
    setOpen: mockSetOpen,
  }),
}));

const mockUseGlobalSearch = jest.fn((_query: string): SearchResult[] => [
  {
    id: "organization:org_second",
    category: "organization",
    icon: Boxes,
    title: "Second Org",
    subtitle: "Switch organization > org_second",
    href: "/settings/organization",
    onSelect: mockOnSelect,
  },
]);
jest.mock("@/hooks/use-global-search", () => ({
  useGlobalSearch: (query: string) => mockUseGlobalSearch(query),
}));

describe("CommandPalette", () => {
  beforeEach(() => {
    window.HTMLElement.prototype.scrollIntoView = jest.fn();
    mockOpen = true;
    mockSetOpen.mockClear();
    mockPush.mockClear();
    mockOnSelect.mockClear();
    mockUseGlobalSearch.mockClear();
  });

  it("runs a search result action directly instead of routing", () => {
    render(<CommandPalette />);

    fireEvent.click(screen.getByRole("option", { name: /Second Org/i }));

    expect(mockSetOpen).toHaveBeenCalledWith(false);
    expect(mockOnSelect).toHaveBeenCalled();
    expect(mockPush).not.toHaveBeenCalled();
  });

  it("does not run global search (and its data fetching) while closed", () => {
    mockOpen = false;
    render(<CommandPalette />);

    // The search UI and all its entity queries are only mounted when open, so
    // navigating to an unrelated page must not trigger any list fetches.
    expect(mockUseGlobalSearch).not.toHaveBeenCalled();
    expect(screen.queryByRole("option", { name: /Second Org/i })).toBeNull();
  });

  it("runs global search once the palette is open", () => {
    render(<CommandPalette />);

    expect(mockUseGlobalSearch).toHaveBeenCalled();
    expect(screen.getByRole("option", { name: /Second Org/i })).toBeInTheDocument();
  });
});

function page(title: string, navigationGroup: string): SearchResult {
  return {
    id: `nav:${title}`,
    category: "navigation",
    icon: Boxes,
    title,
    href: `/${title.toLowerCase()}`,
    navigationGroup,
  };
}

describe("navigation results", () => {
  beforeEach(() => {
    window.HTMLElement.prototype.scrollIntoView = jest.fn();
    mockOpen = true;
    mockPush.mockClear();
    mockSetOpen.mockClear();
  });

  it("groups pages by the current layout and selects across group boundaries", () => {
    mockUseGlobalSearch.mockReturnValue([
      page("Chat", "Chat"),
      page("Sessions", "Operational"),
      page("Agents", "Building"),
    ]);
    render(<CommandPalette />);
    expect(screen.getByRole("group", { name: "Operational" })).toHaveTextContent("Sessions");
    expect(screen.getByRole("group", { name: "Building" })).toHaveTextContent("Agents");
    const input = screen.getByRole("combobox");
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(screen.getByRole("option", { name: "Sessions" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(input).toHaveAttribute(
      "aria-activedescendant",
      screen.getByRole("option", { name: "Sessions" }).id,
    );
    fireEvent.keyDown(input, { key: "Enter" });
    expect(mockPush).toHaveBeenCalledWith("/sessions");
    expect(mockSetOpen).toHaveBeenCalledWith(false);
  });

  it("keeps a usable selection after asynchronous results shrink or arrive", () => {
    mockUseGlobalSearch.mockReturnValue([
      page("Chat", "Chat"),
      page("Sessions", "Operational"),
      page("Agents", "Building"),
    ]);
    const { rerender } = render(<CommandPalette />);
    const input = screen.getByRole("combobox");
    fireEvent.keyDown(input, { key: "ArrowDown" });
    fireEvent.keyDown(input, { key: "ArrowDown" });
    mockUseGlobalSearch.mockReturnValue([page("Chat", "Chat"), page("Sessions", "Operational")]);
    rerender(<CommandPalette />);
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(screen.getByRole("option", { name: "Chat" })).toHaveAttribute("aria-selected", "true");
    mockUseGlobalSearch.mockReturnValue([]);
    rerender(<CommandPalette />);
    fireEvent.keyDown(input, { key: "ArrowDown" });
    expect(input).not.toHaveAttribute("aria-activedescendant");
    mockUseGlobalSearch.mockReturnValue([page("Agents", "Building")]);
    rerender(<CommandPalette />);
    fireEvent.keyDown(input, { key: "Enter" });
    expect(mockPush).toHaveBeenCalledWith("/agents");
  });
});
