import { fireEvent, render, screen } from "@testing-library/react";
import type React from "react";
import { Boxes } from "lucide-react";
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
      Popup: (props: React.HTMLAttributes<HTMLDivElement>) =>
        ReactRuntime.createElement("div", props),
    },
  };
});

jest.mock("@/hooks/use-command-palette", () => ({
  useCommandPalette: () => ({
    open: true,
    setOpen: mockSetOpen,
  }),
}));

jest.mock("@/hooks/use-global-search", () => ({
  useGlobalSearch: () => [
    {
      id: "organization:org_second",
      category: "organization",
      icon: Boxes,
      title: "Second Org",
      subtitle: "Switch organisation > org_second",
      href: "/settings/organization",
      onSelect: mockOnSelect,
    },
  ],
}));

describe("CommandPalette", () => {
  beforeEach(() => {
    window.HTMLElement.prototype.scrollIntoView = jest.fn();
    mockSetOpen.mockClear();
    mockPush.mockClear();
    mockOnSelect.mockClear();
  });

  it("runs a search result action directly instead of routing", () => {
    render(<CommandPalette />);

    fireEvent.click(screen.getByRole("button", { name: /Second Org/i }));

    expect(mockSetOpen).toHaveBeenCalledWith(false);
    expect(mockOnSelect).toHaveBeenCalled();
    expect(mockPush).not.toHaveBeenCalled();
  });
});
