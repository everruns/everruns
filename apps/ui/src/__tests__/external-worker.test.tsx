import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ExternalWorker } from "@/components/agents/external-worker";
import { api } from "@/lib/api/client";

jest.mock("@/providers/locale-provider", () => ({
  useUiTranslations: () => (key: string) => key,
}));
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: "org_fixture" }, isLoading: false }),
}));
jest.mock("@/lib/api/client", () => ({
  api: { get: jest.fn(), post: jest.fn(), delete: jest.fn() },
}));

const endpoint = "https://agent.example/";
const secret = "synthetic-endpoint-secret-of-sufficient-length";
const path = "/v1/agents/agent_fixture/external-worker";
let binding: { endpoint_url: string; enabled: boolean } | null;

function show(disabled = false) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  return render(
    <QueryClientProvider client={client}>
      <ExternalWorker agentId="agent_fixture" disabled={disabled} />
    </QueryClientProvider>,
  );
}

beforeEach(() => {
  jest.clearAllMocks();
  binding = null;
  jest.mocked(api.get).mockImplementation(async () => ({ data: binding }) as never);
  jest.mocked(api.post).mockImplementation(async () => {
    binding = { endpoint_url: endpoint, enabled: true };
    return { data: binding } as never;
  });
});
afterEach(cleanup);

test("verified registration clears the secret and refreshes public status", async () => {
  show();
  const configure = screen.getByRole("button", { name: "Configure execution" });
  await waitFor(() => expect(configure).toBeEnabled());
  fireEvent.click(configure);
  fireEvent.change(screen.getByLabelText("Worker HTTPS endpoint"), { target: { value: endpoint } });
  fireEvent.change(screen.getByLabelText("Endpoint secret"), { target: { value: secret } });
  fireEvent.click(screen.getByRole("button", { name: "Verify and connect" }));
  await screen.findByText("External worker");
  expect(api.post).toHaveBeenCalledWith(path, { endpoint_url: endpoint, endpoint_secret: secret });
  expect(screen.queryByLabelText("Endpoint secret")).not.toBeInTheDocument();
  expect(screen.getByText(endpoint)).toBeInTheDocument();
  fireEvent.click(configure);
  expect(screen.getByLabelText("Endpoint secret")).toHaveValue("");
});

test("failed ownership verification stays visible without enabling the binding", async () => {
  jest.mocked(api.post).mockRejectedValue(new Error("Endpoint ownership verification failed"));
  show();
  const configure = screen.getByRole("button", { name: "Configure execution" });
  await waitFor(() => expect(configure).toBeEnabled());
  fireEvent.click(configure);
  fireEvent.change(screen.getByLabelText("Worker HTTPS endpoint"), { target: { value: endpoint } });
  fireEvent.change(screen.getByLabelText("Endpoint secret"), { target: { value: secret } });
  fireEvent.click(screen.getByRole("button", { name: "Verify and connect" }));
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "Endpoint ownership verification failed",
  );
  expect(screen.queryByText("External worker")).not.toBeInTheDocument();
  expect(screen.getByLabelText("Endpoint secret")).toHaveAttribute("type", "password");
  fireEvent.click(configure);
  fireEvent.click(configure);
  expect(screen.getByLabelText("Endpoint secret")).toHaveValue("");
});

test("a read-only viewer can inspect status but cannot open credential controls", async () => {
  binding = { endpoint_url: endpoint, enabled: true };
  show(true);
  await screen.findByText(endpoint);
  expect(screen.getByRole("button", { name: "Configure execution" })).toBeDisabled();
  expect(screen.queryByLabelText("Endpoint secret")).not.toBeInTheDocument();
  expect(api.post).not.toHaveBeenCalled();
});
