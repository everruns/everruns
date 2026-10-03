import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { usePlatformChatThread } from "@/hooks/use-platform-chat-thread";
import { api } from "@/lib/api/client";
jest.mock("@/lib/api/client", () => ({ api: { post: jest.fn() } }));
let mockOrg = "org_a";
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => ({ currentOrg: { public_id: mockOrg }, isLoading: false }),
}));
function Probe({ ensure }: { ensure?: boolean }) {
  const { thread, error } = usePlatformChatThread({ ensure });
  return <span>{error?.message ?? thread?.id ?? "waiting"}</span>;
}
const mockPost = jest.mocked(api.post);
function client() {
  return new QueryClient({ defaultOptions: { queries: { retry: false } } });
}
beforeEach(() => {
  jest.clearAllMocks();
  mockOrg = "org_a";
  mockPost.mockResolvedValue({ data: { id: "ses_main" } });
});
it("resolves the permanent chat directly and deduplicates simultaneous consumers", async () => {
  render(
    <QueryClientProvider client={client()}>
      <Probe />
      <Probe />
    </QueryClientProvider>,
  );
  await waitFor(() => expect(screen.getAllByText("ses_main")).toHaveLength(2));
  expect(mockPost).toHaveBeenCalledTimes(1);
  expect(mockPost).toHaveBeenCalledWith("/v1/sessions/platform-chat", {});
});
it("does not create with ensure disabled", () => {
  render(
    <QueryClientProvider client={client()}>
      <Probe ensure={false} />
    </QueryClientProvider>,
  );
  expect(mockPost).not.toHaveBeenCalled();
});
it("isolates permanent conversations by organization", async () => {
  const queryClient = client();
  const { rerender } = render(
    <QueryClientProvider client={queryClient}>
      <Probe />
    </QueryClientProvider>,
  );
  await screen.findByText("ses_main");
  mockOrg = "org_b";
  mockPost.mockResolvedValue({ data: { id: "ses_other" } });
  rerender(
    <QueryClientProvider client={queryClient}>
      <Probe />
    </QueryClientProvider>,
  );
  await screen.findByText("ses_other");
  expect(mockPost).toHaveBeenCalledTimes(2);
});
it("exposes failures instead of creating a replacement conversation", async () => {
  mockPost.mockRejectedValue(new Error("unavailable"));
  render(
    <QueryClientProvider client={client()}>
      <Probe />
    </QueryClientProvider>,
  );
  await screen.findByText("unavailable");
  expect(mockPost).toHaveBeenCalledTimes(1);
});
