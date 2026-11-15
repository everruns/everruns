import { act, fireEvent, render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { useState } from "react";
import { McpAppProvider } from "../provider";
import { connectWorkspace, workspaceRequest, type WorkspaceInit } from "../transport";

const router = { replace: jest.fn() };
jest.mock("next/navigation", () => ({
  useRouter: () => router,
  usePathname: () => "/mcp-app/sessions/session_one/approvals",
}));
jest.mock("../transport", () => ({
  ...jest.requireActual("../transport"),
  connectWorkspace: jest.fn(),
  workspaceRequest: jest.fn(),
  installWorkspaceTransport: () => () => {},
}));

function Draft() {
  const [value, setValue] = useState("");
  return <input aria-label="Answer" value={value} onChange={(e) => setValue(e.target.value)} />;
}

it("preserves drafts and the selected tab on repeated entry results and host context changes", async () => {
  let initialize!: (init: WorkspaceInit) => void;
  let theme!: (theme: "light" | "dark") => void;
  jest.mocked(connectWorkspace).mockImplementation((onInit, onTheme) => {
    initialize = onInit;
    theme = onTheme!;
    return () => {};
  });
  jest.mocked(workspaceRequest).mockResolvedValue({
    user: { id: "user_one" },
    organization: { public_id: "org_one", role: "owner" },
    feature_flags: {},
    read_only: false,
  });
  const client = new QueryClient();
  const clear = jest.spyOn(client, "clear");
  render(
    <QueryClientProvider client={client}>
      <McpAppProvider>
        <Draft />
      </McpAppProvider>
    </QueryClientProvider>,
  );
  const entry = { view: { kind: "session", id: "session_one" }, organization_id: "org_one" };
  await act(async () => initialize(entry));
  fireEvent.change(screen.getByLabelText("Answer"), { target: { value: "Use staging" } });
  // A needless second bootstrap would remain pending and unmount the draft.
  jest.mocked(workspaceRequest).mockImplementation(() => new Promise(() => {}));
  await act(async () => {
    theme("dark");
    initialize({ ...entry });
  });
  expect(screen.getByLabelText("Answer")).toHaveValue("Use staging");
  expect(router.replace).toHaveBeenCalledTimes(1);
  expect(workspaceRequest).toHaveBeenCalledTimes(1);
  expect(clear).toHaveBeenCalledTimes(1);

  jest.mocked(workspaceRequest).mockResolvedValueOnce({
    user: { id: "user_one" },
    organization: { public_id: "org_two", role: "owner" },
    feature_flags: {},
    read_only: false,
  });
  await act(async () => initialize({ ...entry, organization_id: "org_two" }));
  expect(workspaceRequest).toHaveBeenCalledTimes(2);
  expect(clear).toHaveBeenCalledTimes(2);
  expect(screen.getByLabelText("Answer")).toHaveValue("");
});
