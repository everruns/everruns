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
function Probe({ flag }: { flag: "monitoring" | "extractions" }) {
  const { flags, isLoading } = useFeatureFlagsState();
  return <span aria-busy={isLoading}>{flags[flag] ? "enabled" : "disabled"}</span>;
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
  jest
    .mocked(getFeatureFlags)
    .mockResolvedValue({ monitoring: true, extractions: true } as FeatureFlags);
  jest
    .mocked(getOrgFeatureFlags)
    .mockImplementation(
      async (id) =>
        ({ monitoring: id === "opted-in", extractions: id === "opted-in" }) as FeatureFlags,
    );
});
describe.each(["monitoring", "extractions"] as const)("%s adoption", (flag) => {
  it("requires an organization even when the deployment makes the feature available", async () => {
    render(<Probe flag={flag} />, { wrapper });
    await waitFor(() => expect(screen.getByText("disabled")).toHaveAttribute("aria-busy", "false"));
  });

  it("requires the org-effective opt-in and resets on organization switch", async () => {
    mockOrg = { public_id: "opted-in" };
    const { rerender } = render(<Probe flag={flag} />, { wrapper });
    expect(screen.getByText("disabled")).toBeInTheDocument();
    expect(await screen.findByText("enabled")).toBeInTheDocument();
    mockOrg = { public_id: "not-enrolled" };
    rerender(<Probe flag={flag} />);
    expect(screen.getByText("disabled")).toBeInTheDocument();
    await waitFor(() => expect(screen.getByText("disabled")).toHaveAttribute("aria-busy", "false"));
  });
});
