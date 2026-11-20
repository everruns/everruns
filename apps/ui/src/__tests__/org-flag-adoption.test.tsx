import { render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { FeatureFlagsProvider, useFeatureFlagsState } from "@/providers/feature-flags-provider";
import { getFeatureFlags, getOrgFeatureFlags } from "@/lib/api/feature-flags";
import type { FeatureFlags } from "@/lib/api/types";

let mockOrg: { public_id: string } | undefined;
jest.mock("@/providers/org-provider", () => ({ useOrg: () => ({ currentOrg: mockOrg }) }));
jest.mock("@/lib/api/feature-flags", () => ({
  getFeatureFlags: jest.fn(),
  getOrgFeatureFlags: jest.fn(),
}));
function Probe() {
  const { flags, isLoading } = useFeatureFlagsState();
  return <span aria-busy={isLoading}>{flags.extractions ? "enabled" : "disabled"}</span>;
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
  jest.mocked(getFeatureFlags).mockResolvedValue({ extractions: true } as FeatureFlags);
  jest
    .mocked(getOrgFeatureFlags)
    .mockImplementation(async (id) => ({ extractions: id === "opted-in" }) as FeatureFlags);
});
describe("extractions adoption", () => {
  it("requires an organization even when the deployment makes the feature available", async () => {
    render(<Probe />, { wrapper });
    await waitFor(() => expect(screen.getByText("disabled")).toHaveAttribute("aria-busy", "false"));
  });

  it("requires the org-effective opt-in and resets on organization switch", async () => {
    mockOrg = { public_id: "opted-in" };
    const { rerender } = render(<Probe />, { wrapper });
    expect(screen.getByText("disabled")).toBeInTheDocument();
    expect(await screen.findByText("enabled")).toBeInTheDocument();
    mockOrg = { public_id: "not-enrolled" };
    rerender(<Probe />);
    expect(screen.getByText("disabled")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("disabled")).toHaveAttribute("aria-busy", "false"));
  });
});
