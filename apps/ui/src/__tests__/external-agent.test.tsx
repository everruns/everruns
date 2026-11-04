import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import {
  ExternalAgentPane,
  ExternalAgentSheet,
  externalAgentSummary,
  generateEndpointSecret,
  type ExternalAgentStatus,
} from "@/components/agents/external-agent";
import { api } from "@/lib/api/client";

jest.mock("@/providers/locale-provider", () => ({
  useUiTranslations: () => (key: string) => key,
  useLocale: () => ({ displayLocale: "en" }),
}));
jest.mock("@/components/ui/confirmation-provider", () => ({
  useConfirmation: () => async () => true,
}));
jest.mock("@/lib/api/client", () => ({
  api: { get: jest.fn(), post: jest.fn(), delete: jest.fn() },
}));

const endpoint = "https://agent.example/";
const path = "/v1/agents/agent_fixture/external-worker";
const connected = {
  endpoint_url: endpoint,
  enabled: true,
  last_error: null,
  last_invoked_at: null,
};

function sheet(status: ExternalAgentStatus, canManage = true) {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false, gcTime: 0 } },
  });
  return render(
    <QueryClientProvider client={client}>
      <ExternalAgentSheet
        agentId="agent_fixture"
        open
        onOpenChange={() => {}}
        status={status}
        canManage={canManage}
      />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  jest.mocked(api.post).mockResolvedValue({ data: connected } as never);
  jest.mocked(api.delete).mockResolvedValue({ data: null } as never);
});
afterEach(cleanup);

test("generated secrets are long, URL-safe and unique", () => {
  const a = generateEndpointSecret();
  expect(a).toMatch(/^[A-Za-z0-9_-]{48}$/);
  expect(generateEndpointSecret()).not.toBe(a);
});

test("summary says whether the endpoint is connected", () => {
  const ui = (key: string) => key;
  expect(externalAgentSummary(null, ui)).toBe("Not connected");
  expect(externalAgentSummary(connected, ui)).toBe("Connected");
  expect(externalAgentSummary({ ...connected, enabled: false }, ui)).toBe("Disconnected");
});

test("setup generates a secret and sends it only on Verify and connect", async () => {
  sheet({ available: true, endpoint: null });
  const secret = screen.getByLabelText("Install an endpoint secret");
  expect(secret).toHaveAttribute("type", "password");
  fireEvent.click(screen.getByRole("button", { name: "Generate" }));
  const value = (secret as HTMLInputElement).value;
  expect(value).toHaveLength(48);
  expect(api.post).not.toHaveBeenCalled();
  fireEvent.change(screen.getByLabelText("Endpoint URL"), {
    target: { value: endpoint },
  });
  fireEvent.click(screen.getByRole("button", { name: "Verify and connect" }));
  await waitFor(() =>
    expect(api.post).toHaveBeenCalledWith(path, {
      endpoint_url: endpoint,
      endpoint_secret: value,
    }),
  );
});

test("failed verification stays visible", async () => {
  jest.mocked(api.post).mockRejectedValue(new Error("Endpoint ownership verification failed"));
  sheet({ available: true, endpoint: null });
  fireEvent.click(screen.getByRole("button", { name: "Generate" }));
  fireEvent.change(screen.getByLabelText("Endpoint URL"), {
    target: { value: endpoint },
  });
  fireEvent.click(screen.getByRole("button", { name: "Verify and connect" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Endpoint ownership verification failed",
  );
});

test("a connected endpoint shows status and disconnects after confirmation", async () => {
  sheet({ available: true, endpoint: connected });
  expect(screen.getByText(endpoint)).toBeInTheDocument();
  expect(screen.queryByLabelText("Install an endpoint secret")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Disconnect endpoint" }));
  await waitFor(() => expect(api.delete).toHaveBeenCalledWith(path));
  fireEvent.click(screen.getByRole("button", { name: "Update connection" }));
  expect(screen.getByLabelText("Endpoint URL")).toHaveValue(endpoint);
  expect(screen.getByLabelText("Install an endpoint secret")).toHaveValue("");
});

test("a viewer without permission sees status but no credential controls", () => {
  sheet({ available: true, endpoint: connected }, false);
  expect(screen.getByText(endpoint)).toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Update connection" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "Disconnect endpoint" })).not.toBeInTheDocument();
  expect(screen.queryByLabelText("Install an endpoint secret")).not.toBeInTheDocument();
});

test("the pane offers Reconnect for a disconnected endpoint", () => {
  const manage = jest.fn();
  render(<ExternalAgentPane endpoint={{ ...connected, enabled: false }} onManage={manage} />);
  expect(screen.getByText("Disconnected")).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Reconnect" }));
  expect(manage).toHaveBeenCalled();
});
