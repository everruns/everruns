import { act, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import type { ReactNode } from "react";
import { PageNavigationRail } from "@/components/chat-mode/page-navigation-rail";
import {
  defaultBuildingNavigation,
  defaultNavigationSections,
  type NavigationSection,
} from "@/lib/navigation";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({
    children,
    prefetch: _prefetch,
    ...props
  }: {
    children: ReactNode;
    prefetch?: boolean;
  }) => <a {...props}>{children}</a>,
}));

const sections: NavigationSection[] = [{ label: "Building", items: defaultBuildingNavigation }];

it("provides a short explanation for every default destination", () => {
  for (const item of defaultNavigationSections.flatMap((section) => section.items)) {
    expect({ name: item.name, description: item.description }).toEqual({
      name: item.name,
      description: expect.stringMatching(/\S/),
    });
  }
});

it("explains an unfamiliar icon on keyboard focus and dismisses with Escape", async () => {
  render(
    <PageNavigationRail open pathname="/agents/example" sections={sections} onClose={jest.fn()} />,
  );
  fireEvent.keyDown(document, { key: "Tab" });
  act(() => screen.getByRole("link", { name: "Harnesses" }).focus());
  expect(screen.getByRole("link", { name: "Harnesses" })).toHaveFocus();
  const hint = await screen.findByRole("tooltip");
  expect(hint).toHaveTextContent("Harnesses");
  expect(hint).toHaveTextContent("Reusable instructions and tools for agents.");
  expect(screen.getByRole("link", { name: "Harnesses" })).toHaveAccessibleDescription(
    "Harnesses Reusable instructions and tools for agents.",
  );
  fireEvent.keyDown(document, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("tooltip")).not.toBeInTheDocument());
});

it("opens a labeled, grouped menu without requiring hover", async () => {
  render(
    <PageNavigationRail open pathname="/agents/example" sections={sections} onClose={jest.fn()} />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Browse pages" }));
  const menu = await screen.findByRole("menu");
  expect(within(menu).getByText("Building")).toBeVisible();
  expect(within(menu).getByText("Reusable instructions and tools for agents.")).toBeVisible();
  expect(within(menu).getByRole("menuitem", { name: "Agents" })).toHaveAttribute("href", "/agents");
  expect(within(menu).getByRole("menuitem", { name: "Agents" })).toHaveAttribute(
    "aria-current",
    "page",
  );
  fireEvent.keyDown(menu, { key: "Escape" });
  await waitFor(() => expect(screen.queryByRole("menu")).not.toBeInTheDocument());
  expect(screen.getByRole("button", { name: "Browse pages" })).toHaveFocus();
});

it("removes open overlays when Chat closes", async () => {
  const props = { pathname: "/agents/example", sections, onClose: jest.fn() };
  const { rerender } = render(<PageNavigationRail open {...props} />);
  fireEvent.click(screen.getByRole("button", { name: "Browse pages" }));
  await screen.findByRole("menu");
  rerender(<PageNavigationRail open={false} {...props} />);
  expect(screen.queryByRole("menu")).not.toBeInTheDocument();
  expect(screen.queryByRole("tooltip")).not.toBeInTheDocument();
});
