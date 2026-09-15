import { render, screen } from "@testing-library/react";

import { AppRetirementNotice } from "@/components/apps/app-retirement-notice";

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href }: { children: React.ReactNode; href: string }) => (
    <a href={href}>{children}</a>
  ),
}));

describe("AppRetirementNotice", () => {
  it("explains the transition and links every bound agent", () => {
    render(
      <AppRetirementNotice
        agents={[
          { id: "agent-1", name: "Support Agent" },
          { id: "agent-2", name: "Sales Agent" },
        ]}
      />,
    );

    expect(screen.getByText("Apps are moving to agent-owned endpoints")).toBeInTheDocument();
    expect(screen.getByText(/Existing Apps and their channels keep working/)).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Support Agent" })).toHaveAttribute(
      "href",
      "/agents/agent-1",
    );
    expect(screen.getByRole("link", { name: "Sales Agent" })).toHaveAttribute(
      "href",
      "/agents/agent-2",
    );
  });
});
