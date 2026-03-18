import { render, screen } from "@testing-library/react";
import React from "react";
import { SelectTrigger } from "@/components/ui/select";

jest.mock("@base-ui/react/select", () => {
  const passthrough = React.forwardRef(function Passthrough(
    {
      children,
      render,
      ...props
    }: {
      children?: React.ReactNode;
      render?: (...args: unknown[]) => React.ReactNode;
      [key: string]: unknown;
    },
    ref: React.Ref<HTMLElement>,
  ) {
    const content = typeof render === "function" ? render({}, { value: "mock-value" }) : children;
    return (
      <div ref={ref as React.Ref<HTMLDivElement>} {...props}>
        {content}
      </div>
    );
  });

  return {
    Select: {
      Root: passthrough,
      Group: passthrough,
      Value: passthrough,
      Trigger: React.forwardRef(function Trigger(
        { children, className, ...props }: React.ButtonHTMLAttributes<HTMLButtonElement>,
        ref: React.Ref<HTMLButtonElement>,
      ) {
        return (
          <button ref={ref} className={className} {...props}>
            {children}
          </button>
        );
      }),
      Icon: ({ render }: { render?: React.ReactNode }) => <>{render ?? null}</>,
      Portal: ({ children }: { children?: React.ReactNode }) => <>{children}</>,
      Positioner: passthrough,
      Popup: passthrough,
      GroupLabel: passthrough,
      Item: passthrough,
      ItemIndicator: passthrough,
      ItemText: passthrough,
      Separator: passthrough,
    },
  };
});

describe("SelectTrigger", () => {
  it("resets native button appearance for shared pickers", () => {
    render(<SelectTrigger>Default</SelectTrigger>);

    expect(screen.getByRole("button")).toHaveClass("appearance-none");
  });
});
