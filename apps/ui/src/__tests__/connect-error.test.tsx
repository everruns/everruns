import { act, fireEvent, render, screen } from "@testing-library/react";
import { ConnectErrorBanner } from "@/components/connections/connect-error-banner";
import { connectErrorMessage } from "@/lib/connect-error";

describe("connectErrorMessage", () => {
  it("names each failure in plain words", () => {
    expect(connectErrorMessage("blocked_by_network_policy")).toBe(
      "Couldn't connect: this server's host isn't on the organization's allowed network list.",
    );
    expect(connectErrorMessage("provider_unreachable")).toBe(
      "Couldn't reach the server's sign-in service. Try again later.",
    );
    expect(connectErrorMessage("provider_refused")).toBe("The provider declined the sign-in.");
    expect(connectErrorMessage("failed")).toBe("Couldn't connect. Try again.");
    expect(connectErrorMessage("<script>")).toBe("Couldn't connect. Try again.");
  });
});

describe("ConnectErrorBanner", () => {
  afterEach(() => {
    window.history.replaceState(null, "", "/");
  });

  it("shows the returned error and strips only the connect params", () => {
    window.history.replaceState(
      null,
      "",
      "/agents/agent_1?tab=mcp&connect_error=provider_unreachable&provider=mcp_oauth_x",
    );
    render(<ConnectErrorBanner />);

    expect(screen.getByRole("alert")).toHaveTextContent(
      "Couldn't reach the server's sign-in service. Try again later.",
    );
    expect(`${window.location.pathname}${window.location.search}`).toBe("/agents/agent_1?tab=mcp");

    act(() => {
      fireEvent.click(screen.getByRole("button", { name: "Dismiss" }));
    });
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
  });

  it("renders nothing without a connect error", () => {
    window.history.replaceState(null, "", "/agents/agent_1?tab=mcp&connected=mcp_oauth_x");
    render(<ConnectErrorBanner />);

    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(window.location.search).toBe("?tab=mcp&connected=mcp_oauth_x");
  });
});
