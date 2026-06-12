import * as React from "react";
import { render, screen } from "@testing-library/react";
import {
  DropdownMenuSub,
  DropdownMenuSubContent,
  DropdownMenuSubTrigger,
} from "@/components/ui/dropdown-menu";

jest.mock("@base-ui/react/menu", () => {
  const ReactActual = jest.requireActual<typeof import("react")>("react");

  type PassthroughProps = React.HTMLAttributes<HTMLElement> & {
    children?: React.ReactNode;
    sideOffset?: number;
  };

  const divPassthrough = ReactActual.forwardRef<HTMLElement, PassthroughProps>(
    function DivPassthrough({ children, sideOffset, ...props }, ref) {
      return (
        <div ref={ref as React.Ref<HTMLDivElement>} data-side-offset={sideOffset} {...props}>
          {children}
        </div>
      );
    },
  );

  const buttonPassthrough = ReactActual.forwardRef<
    HTMLButtonElement,
    React.ButtonHTMLAttributes<HTMLButtonElement>
  >(function ButtonPassthrough({ children, ...props }, ref) {
    return (
      <button ref={ref} type="button" {...props}>
        {children}
      </button>
    );
  });

  return {
    Menu: {
      Root: divPassthrough,
      Portal: divPassthrough,
      Trigger: buttonPassthrough,
      Positioner: divPassthrough,
      Popup: divPassthrough,
      Group: divPassthrough,
      Item: divPassthrough,
      CheckboxItem: divPassthrough,
      CheckboxItemIndicator: divPassthrough,
      RadioGroup: divPassthrough,
      RadioItem: divPassthrough,
      RadioItemIndicator: divPassthrough,
      GroupLabel: divPassthrough,
      Separator: divPassthrough,
      SubmenuRoot: divPassthrough,
      SubmenuTrigger: buttonPassthrough,
    },
  };
});

describe("DropdownMenuSubContent", () => {
  it("renders submenu content inside a portal and positioner", () => {
    render(
      <DropdownMenuSub>
        <DropdownMenuSubTrigger>Notifications</DropdownMenuSubTrigger>
        <DropdownMenuSubContent>Submenu content</DropdownMenuSubContent>
      </DropdownMenuSub>,
    );

    const submenuContent = screen.getByText("Submenu content");
    const positioner = submenuContent.closest('[data-slot="dropdown-menu-positioner"]');
    const portal = submenuContent.closest('[data-slot="dropdown-menu-portal"]');

    expect(positioner).not.toBeNull();
    expect(positioner).toHaveClass("z-50");
    expect(portal).not.toBeNull();
    expect(portal).toContainElement(positioner as HTMLElement | null);
  });
});
