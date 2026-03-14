import { renderHook, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { AuthProvider, useAuth } from "@/providers/auth-provider";
import { ApiError } from "@/lib/api/client";

// Mock the auth hooks used by AuthProvider
const mockLogoutMutateAsync = jest.fn();
const mockUseAuthConfig = jest.fn();
const mockUseCurrentUser = jest.fn();
jest.mock("@/hooks/use-auth", () => ({
  useAuthConfig: () => mockUseAuthConfig(),
  useCurrentUser: () => mockUseCurrentUser(),
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
    mockUseAuthConfig.mockReturnValue({
      data: { mode: "password", oauth_providers: [] },
      isLoading: false,
      error: null,
    });
    mockUseCurrentUser.mockReturnValue({
      data: { id: "user-1", email: "test@example.com", name: "Test User", roles: ["user"] },
      isLoading: false,
      error: null,
    });
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

  it("marks auth as unavailable when auth config bootstrap fails", () => {
    mockUseAuthConfig.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new Error("config failed"),
    });
    mockUseCurrentUser.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: null,
    });

    const { result } = renderHook(() => useAuth(), { wrapper: makeWrapper() });

    expect(result.current.authUnavailable).toBe(true);
    expect(result.current.isAuthenticated).toBe(false);
  });

  it("marks auth as unavailable when current user bootstrap fails with a transport error", () => {
    mockUseCurrentUser.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new ApiError(503, "Service Unavailable", "backend down"),
    });

    const { result } = renderHook(() => useAuth(), { wrapper: makeWrapper() });

    expect(result.current.authUnavailable).toBe(true);
    expect(result.current.isAuthenticated).toBe(false);
  });

  it("keeps unauthenticated 401 distinct from auth-unavailable failures", () => {
    mockUseCurrentUser.mockReturnValue({
      data: undefined,
      isLoading: false,
      error: new ApiError(401, "Unauthorized", "not logged in"),
    });

    const { result } = renderHook(() => useAuth(), { wrapper: makeWrapper() });

    expect(result.current.authUnavailable).toBe(false);
    expect(result.current.requiresAuth).toBe(true);
    expect(result.current.isAuthenticated).toBe(false);
  });
});
