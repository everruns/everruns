import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { AgentCredentialsPanel } from "@/components/agents/agent-credentials-panel";
import { ScopedCredentialsPanel } from "@/components/credentials/scoped-credentials-panel";
import * as api from "@/lib/api/credential-bindings";
import type { ScopedCredential } from "@/lib/api/credential-bindings";
jest.mock("@/lib/api/credential-bindings");
let mockOrg = "org_a";
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: mockOrg }, hasRole: () => true }),
}));
jest.mock("@/components/agent/agent-select", () => ({
  AgentSelect: () => <div>Agent picker</div>,
}));
const credential: ScopedCredential = {
  id: "binding-1",
  name: "MAIL_TOKEN",
  label: "Mail account",
  kind: "http",
  binding: { url: "https://mail.example/api", auth: { type: "bearer" } },
  mcp_server_url: null,
  tool_name: null,
  parameter_name: null,
  scope: "agent",
  configured: false,
  expires_at: null,
  revision: 7,
  last_used_at: null,
  last_used_agent_id: null,
  last_used_agent_name: null,
  source_connection_id: null,
};
function setup(element = <AgentCredentialsPanel agentId="agent-1" />) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  return {
    client,
    ...render(<QueryClientProvider client={client}>{element}</QueryClientProvider>),
  };
}
beforeEach(() => {
  jest.resetAllMocks();
  mockOrg = "org_a";
  jest.mocked(api.listScopedCredentials).mockResolvedValue([credential]);
  jest.mocked(api.saveCredentialValue).mockResolvedValue();
  jest.mocked(api.createScopedCredential).mockResolvedValue({ id: "new-binding" });
  jest.mocked(api.revokeScopedCredential).mockResolvedValue();
});
it("shows exact destination and placement without prefilling a secret", async () => {
  setup();
  await screen.findByText("Mail account");
  expect(screen.getByText("https://mail.example/api")).toBeInTheDocument();
  expect(screen.getByText("Authorization: Bearer")).toBeInTheDocument();
  expect(screen.getByText("$MAIL_TOKEN")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Add value" }));
  const input = screen.getByLabelText("Credential value") as HTMLInputElement;
  expect(input.type).toBe("password");
  expect(input.autocomplete).toBe("new-password");
  expect(input.value).toBe("");
});
it("clears plaintext on success without caching it", async () => {
  const { client } = setup();
  await screen.findByText("Mail account");
  fireEvent.click(screen.getByRole("button", { name: "Add value" }));
  const secret = "ui-secret-sentinel";
  fireEvent.change(screen.getByLabelText("Credential value"), { target: { value: secret } });
  fireEvent.click(screen.getByRole("button", { name: "Save value" }));
  await waitFor(() =>
    expect(api.saveCredentialValue).toHaveBeenCalledWith("binding-1", secret, 7, undefined),
  );
  await waitFor(() => expect(screen.queryByDisplayValue(secret)).not.toBeInTheDocument());
  expect(document.body.textContent).not.toContain(secret);
  expect(
    JSON.stringify(
      client
        .getQueryCache()
        .getAll()
        .map((q) => q.state.data),
    ),
  ).not.toContain(secret);
  expect(client.getMutationCache().getAll()).toHaveLength(0);
});
it("clears plaintext after a failed save and explains recovery", async () => {
  jest
    .mocked(api.saveCredentialValue)
    .mockRejectedValue(new Error("Credential changed; refresh before replacing it"));
  setup();
  await screen.findByText("Mail account");
  fireEvent.click(screen.getByRole("button", { name: "Add value" }));
  fireEvent.change(screen.getByLabelText("Credential value"), {
    target: { value: "failed-secret-sentinel" },
  });
  fireEvent.click(screen.getByRole("button", { name: "Save value" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Credential changed");
  expect(screen.getByLabelText("Credential value")).toHaveValue("");
});
it("declares metadata separately from the secure value step", async () => {
  setup();
  await screen.findByText("Mail account");
  fireEvent.click(screen.getByRole("button", { name: "Add binding" }));
  fireEvent.change(screen.getByLabelText("Label"), { target: { value: "Another service" } });
  fireEvent.change(screen.getByLabelText("Reference name"), { target: { value: "OTHER_TOKEN" } });
  fireEvent.change(screen.getByLabelText("Allowed HTTPS URL"), {
    target: { value: "https://other.example/v1" },
  });
  expect(
    screen.getByText(/web_fetch and Bashkit send Authorization: Bearer \$OTHER_TOKEN/),
  ).toBeInTheDocument();
  expect(
    screen.getByText(/Daytona, E2B, and Modal commands run without this credential/),
  ).toBeInTheDocument();
  expect(screen.queryByLabelText("Credential value")).not.toBeInTheDocument();
  fireEvent.submit(screen.getByLabelText("Label").closest("form")!);
  await waitFor(() =>
    expect(api.createScopedCredential).toHaveBeenCalledWith(
      { scope: "agent", owner_id: "agent-1", usage: "service" },
      {
        name: "OTHER_TOKEN",
        label: "Another service",
        binding: { url: "https://other.example/v1", auth: { type: "bearer" } },
      },
    ),
  );
  expect(api.saveCredentialValue).not.toHaveBeenCalled();
});
it("requires confirmation and the observed revision for revocation", async () => {
  setup();
  await screen.findByText("Mail account");
  fireEvent.click(screen.getByRole("button", { name: "Revoke Mail account" }));
  expect(api.revokeScopedCredential).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Revoke credential" }));
  await waitFor(() => expect(api.revokeScopedCredential).toHaveBeenCalledWith("binding-1", 7));
});
it("drops secure input when organization or owner changes", async () => {
  const { rerender, client } = setup(
    <ScopedCredentialsPanel owner={{ scope: "virtual_user", owner_id: "me", usage: "user" }} />,
  );
  await screen.findByText("Mail account");
  fireEvent.click(screen.getByRole("button", { name: "Add value" }));
  fireEvent.change(screen.getByLabelText("Credential value"), {
    target: { value: "old-org-secret" },
  });
  mockOrg = "org_b";
  rerender(
    <QueryClientProvider client={client}>
      <ScopedCredentialsPanel owner={{ scope: "virtual_user", owner_id: "me", usage: "user" }} />
    </QueryClientProvider>,
  );
  await waitFor(() => expect(screen.queryByDisplayValue("old-org-secret")).not.toBeInTheDocument());
  expect(api.saveCredentialValue).not.toHaveBeenCalled();
});
