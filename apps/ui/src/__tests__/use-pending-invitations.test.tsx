import { act, renderHook } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { useAcceptPendingInvitation } from "@/hooks/use-invitations";
import { authKeys } from "@/hooks/use-auth";
import { queryKeys } from "@/lib/query-keys";

jest.mock("@/lib/api/invitations", () => ({
  acceptPendingInvitation: jest.fn(),
  createInvite: jest.fn(),
  listInvites: jest.fn(),
  listPendingInvitations: jest.fn(),
  revokeInvite: jest.fn(),
}));
jest.mock("@/lib/api/users", () => ({
  switchOrg: jest.fn(),
}));

import { acceptPendingInvitation } from "@/lib/api/invitations";
import { switchOrg } from "@/lib/api/users";

const mockAcceptPendingInvitation = jest.mocked(acceptPendingInvitation);
const mockSwitchOrg = jest.mocked(switchOrg);

describe("useAcceptPendingInvitation", () => {
  let queryClient: QueryClient;
  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: { retry: false },
        mutations: { retry: false },
      },
    });
    jest.clearAllMocks();
  });

  it("selects the joined organization before refreshing the user", async () => {
    mockAcceptPendingInvitation.mockResolvedValue({ org_id: "org_acme", role: "member" });
    mockSwitchOrg.mockResolvedValue({ success: true, org_id: "org_acme" });
    const refetch = jest.spyOn(queryClient, "refetchQueries");
    const invalidate = jest.spyOn(queryClient, "invalidateQueries");
    const { result } = renderHook(() => useAcceptPendingInvitation(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync("orginv_one");
    });

    expect(mockAcceptPendingInvitation).toHaveBeenCalledWith("orginv_one");
    expect(mockSwitchOrg).toHaveBeenCalledWith("org_acme");
    expect(refetch).toHaveBeenCalledWith({ queryKey: authKeys.user() });
    expect(invalidate).toHaveBeenCalledWith({ queryKey: queryKeys.invitations.pending() });
  });

  it("refreshes membership when active-organization preference sync fails", async () => {
    mockAcceptPendingInvitation.mockResolvedValue({ org_id: "org_acme", role: "member" });
    mockSwitchOrg.mockRejectedValue(new Error("cookie unavailable"));
    const refetch = jest.spyOn(queryClient, "refetchQueries");
    const { result } = renderHook(() => useAcceptPendingInvitation(), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync("orginv_one")).resolves.toEqual({
        org_id: "org_acme",
        role: "member",
      });
    });

    expect(refetch).toHaveBeenCalledWith({ queryKey: authKeys.user() });
  });
});
