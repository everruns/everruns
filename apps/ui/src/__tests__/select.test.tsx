import { render, screen } from "@testing-library/react";
import React from "react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

const rootItemsCalls: Array<Record<string, React.ReactNode> | undefined> = [];

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

  const Root = ({
    items,
    children,
  }: {
    items?: Record<string, React.ReactNode>;
    children?: React.ReactNode;
    [key: string]: unknown;
  }) => {
    rootItemsCalls.push(items);
    return <div data-testid="select-root">{children}</div>;
  };

  return {
    Select: {
      Root,
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

beforeEach(() => {
  rootItemsCalls.length = 0;
});

describe("SelectTrigger", () => {
  it("resets native button appearance for shared pickers", () => {
    render(<SelectTrigger>Default</SelectTrigger>);

    expect(screen.getByRole("button")).toHaveClass("appearance-none");
  });
});

describe("Select wrapper auto-collects display labels", () => {
  // The bug: <SelectValue> showed the raw internal value (e.g. "shared_session")
  // because no items map was forwarded to base-ui. The wrapper now walks
  // <SelectItem> children and forwards a value→label map, so display labels
  // are the default. See knowledge/foundations/code-organization.md.
  it("forwards a value→label map collected from SelectItem children", () => {
    render(
      <Select value="webhook" onValueChange={() => {}}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="slack">Slack</SelectItem>
          <SelectItem value="webhook">Webhook</SelectItem>
        </SelectContent>
      </Select>,
    );

    const items = rootItemsCalls.at(-1);
    expect(items).toBeDefined();
    expect(items?.slack).toBe("Slack");
    expect(items?.webhook).toBe("Webhook");
  });

  it("explicit items prop entries take precedence over collected labels", () => {
    render(
      <Select
        value="shared_session"
        onValueChange={() => {}}
        items={{ shared_session: "Override" }}
      >
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="shared_session">Shared Session</SelectItem>
        </SelectContent>
      </Select>,
    );

    const items = rootItemsCalls.at(-1);
    expect(items?.shared_session).toBe("Override");
  });

  it("descends through wrapper components to find SelectItem children", () => {
    function CustomGroup({ children }: { children: React.ReactNode }) {
      return <>{children}</>;
    }

    render(
      <Select value="a" onValueChange={() => {}}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <CustomGroup>
            <SelectItem value="a">Alpha</SelectItem>
            <SelectItem value="b">Beta</SelectItem>
          </CustomGroup>
        </SelectContent>
      </Select>,
    );

    const items = rootItemsCalls.at(-1);
    expect(items?.a).toBe("Alpha");
    expect(items?.b).toBe("Beta");
  });
});
