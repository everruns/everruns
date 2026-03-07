import { renderHook, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { AuthProvider, useAuth } from "@/providers/auth-provider";

// Mock the auth hooks used by AuthProvider
const mockLogoutMutateAsync = jest.fn();
jest.mock("@/hooks/use-auth", () => ({
  useAuthConfig: () => ({
    data: { mode: "password", oauth_providers: [] },
    isLoading: false,
    error: null,
  }),
  useCurrentUser: () => ({
    data: { id: "user-1", email: "test@example.com", name: "Test User", roles: ["user"] },
    isLoading: false,
    error: null,
  }),
  useLogout: () => ({
    mutateAsync: mockLogoutMutateAsync,
    isPending: false,
  }),
}));

describe("AuthProvider pluggable actions", () => {
  let queryClient: QueryClient;

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });
    jest.clearAllMocks();
    mockLogoutMutateAsync.mockResolvedValue(undefined);
  });

  function makeWrapper(providerProps?: {
    logout?: () => Promise<void>;
    createOrganization?: () => void;
  }) {
    return ({ children }: { children: ReactNode }) => (
      <QueryClientProvider client={queryClient}>
        <AuthProvider {...providerProps}>{children}</AuthProvider>
      </QueryClientProvider>
    );
  }

  it("provides default logout that calls useLogout mutation", async () => {
    const { result } = renderHook(() => useAuth(), { wrapper: makeWrapper() });

    await act(async () => {
      await result.current.logout();
    });

    expect(mockLogoutMutateAsync).toHaveBeenCalledTimes(1);
  });

  it("uses logout override when provided", async () => {
    const customLogout = jest.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => useAuth(), {
      wrapper: makeWrapper({ logout: customLogout }),
    });

    await act(async () => {
      await result.current.logout();
    });

    expect(customLogout).toHaveBeenCalledTimes(1);
    expect(mockLogoutMutateAsync).not.toHaveBeenCalled();
  });

  it("createOrganization is undefined by default", () => {
    const { result } = renderHook(() => useAuth(), { wrapper: makeWrapper() });

    expect(result.current.createOrganization).toBeUndefined();
  });

  it("provides createOrganization override when given", () => {
    const customCreate = jest.fn();
    const { result } = renderHook(() => useAuth(), {
      wrapper: makeWrapper({ createOrganization: customCreate }),
    });

    expect(result.current.createOrganization).toBe(customCreate);
    result.current.createOrganization!();
    expect(customCreate).toHaveBeenCalledTimes(1);
  });

  it("logoutPending is false when using override", () => {
    const customLogout = jest.fn().mockResolvedValue(undefined);
    const { result } = renderHook(() => useAuth(), {
      wrapper: makeWrapper({ logout: customLogout }),
    });

    expect(result.current.logoutPending).toBe(false);
  });
});
