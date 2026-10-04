import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { FeatureFlagsProvider, useFeatureFlag } from "@/providers/feature-flags-provider";
import { getFeatureFlags, getOrgFeatureFlags } from "@/lib/api/feature-flags";
import type { FeatureFlags } from "@/lib/api/types";

let mockOrg: { public_id: string } | undefined;
jest.mock("@/providers/org-provider", () => ({ useOrg: () => ({ currentOrg: mockOrg }) }));
jest.mock("@/lib/api/feature-flags", () => ({
  getFeatureFlags: jest.fn(),
  getOrgFeatureFlags: jest.fn(),
}));
function Probe() {
  return <span>{useFeatureFlag("chat_threads") ? "workspace" : "legacy"}</span>;
}
function wrapper({ children }: { children: React.ReactNode }) {
  return (
    <QueryClientProvider
      client={new QueryClient({ defaultOptions: { queries: { retry: false } } })}
    >
      <FeatureFlagsProvider>{children}</FeatureFlagsProvider>
    </QueryClientProvider>
  );
}
beforeEach(() => {
  jest.clearAllMocks();
  mockOrg = undefined;
  jest.mocked(getFeatureFlags).mockResolvedValue({ chat_threads: true } as FeatureFlags);
  jest
    .mocked(getOrgFeatureFlags)
    .mockImplementation(async (id) => ({ chat_threads: id === "opted-in" }) as FeatureFlags);
});
it("deployment availability does not enroll an organization", async () => {
  render(<Probe />, { wrapper });
  await waitFor(() => expect(getFeatureFlags).toHaveBeenCalled());
  expect(screen.getByText("legacy")).toBeInTheDocument();
});
it("requires the org-effective opt-in and resets on organization switch", async () => {
  mockOrg = { public_id: "opted-in" };
  const { rerender } = render(<Probe />, { wrapper });
  expect(screen.getByText("legacy")).toBeInTheDocument();
  expect(await screen.findByText("workspace")).toBeInTheDocument();
  mockOrg = { public_id: "not-enrolled" };
  rerender(<Probe />);
  expect(screen.getByText("legacy")).toBeInTheDocument();
  await waitFor(() => expect(getOrgFeatureFlags).toHaveBeenCalledWith("not-enrolled"));
  expect(screen.getByText("legacy")).toBeInTheDocument();
});
