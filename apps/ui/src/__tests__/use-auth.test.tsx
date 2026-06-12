import { renderHook, act, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import {
  useLogin,
  useRegister,
  useLogout,
  useCurrentUser,
  usePersonalAccessTokens,
  useCreatePersonalAccessToken,
  useDeletePersonalAccessToken,
  authKeys,
} from "@/hooks/use-auth";
import type { OrganizationMembership } from "@/lib/api/types";

// Mock the API functions
jest.mock("@/lib/api/auth", () => ({
  login: jest.fn(),
  register: jest.fn(),
  logout: jest.fn(),
  getCurrentUser: jest.fn(),
  getAuthConfig: jest.fn(),
  listPersonalAccessTokens: jest.fn(),
  createPersonalAccessToken: jest.fn(),
  deletePersonalAccessToken: jest.fn(),
}));

import * as authApi from "@/lib/api/auth";

const mockLogin = authApi.login as jest.MockedFunction<typeof authApi.login>;
const mockRegister = authApi.register as jest.MockedFunction<typeof authApi.register>;
const mockLogout = authApi.logout as jest.MockedFunction<typeof authApi.logout>;
const mockListPersonalAccessTokens = authApi.listPersonalAccessTokens as jest.MockedFunction<
  typeof authApi.listPersonalAccessTokens
>;
const mockCreatePersonalAccessToken = authApi.createPersonalAccessToken as jest.MockedFunction<
  typeof authApi.createPersonalAccessToken
>;
const mockDeletePersonalAccessToken = authApi.deletePersonalAccessToken as jest.MockedFunction<
  typeof authApi.deletePersonalAccessToken
>;
const mockGetCurrentUser = authApi.getCurrentUser as jest.MockedFunction<
  typeof authApi.getCurrentUser
>;

const mockUseOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => mockUseOrg(),
}));

const DEFAULT_ORG: OrganizationMembership = {
  public_id: "org_default",
  name: "Default Org",
  role: "owner",
};

const SECOND_ORG: OrganizationMembership = {
  public_id: "org_second",
  name: "Second Org",
  role: "owner",
};

describe("Auth Hooks", () => {
  let queryClient: QueryClient;

  const wrapper = ({ children }: { children: ReactNode }) => (
    <QueryClientProvider client={queryClient}>{children}</QueryClientProvider>
  );

  beforeEach(() => {
    queryClient = new QueryClient({
      defaultOptions: {
        queries: {
          retry: false,
        },
        mutations: {
          retry: false,
        },
      },
    });
    jest.clearAllMocks();
    localStorage.clear();
    mockUseOrg.mockReturnValue({
      currentOrg: DEFAULT_ORG,
      isLoading: false,
    });
  });

  // Helper to initialize the user query so refetchQueries has something to refetch
  async function initializeUserQuery(initialUser: unknown = null) {
    mockGetCurrentUser.mockResolvedValueOnce(
      initialUser as Awaited<ReturnType<typeof authApi.getCurrentUser>>,
    );
    const { result } = renderHook(() => useCurrentUser(true), { wrapper });
    // Wait for initial fetch to complete
    await act(async () => {
      await new Promise((resolve) => setTimeout(resolve, 10));
    });
    mockGetCurrentUser.mockClear();
    return result;
  }

  describe("useLogin", () => {
    it("should call login API and refetch user data on success", async () => {
      // Initialize the user query first (simulating app state)
      await initializeUserQuery(null);

      const mockTokenResponse = {
        access_token: "test-token",
        refresh_token: "test-refresh",
        token_type: "Bearer",
        expires_in: 900,
      };
      const mockUser = {
        id: "user-1",
        email: "test@example.com",
        name: "Test User",
        roles: ["user"],
      };

      mockLogin.mockResolvedValueOnce(mockTokenResponse);
      mockGetCurrentUser.mockResolvedValueOnce(mockUser);

      const { result } = renderHook(() => useLogin(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync({
          email: "test@example.com",
          password: "password123",
        });
      });

      expect(mockLogin).toHaveBeenCalledWith({
        email: "test@example.com",
        password: "password123",
      });
      expect(mockGetCurrentUser).toHaveBeenCalled();
    });

    it("should wait for user refetch to complete before mutation resolves", async () => {
      // Initialize the user query first
      mockGetCurrentUser.mockResolvedValueOnce(
        null as unknown as Awaited<ReturnType<typeof authApi.getCurrentUser>>,
      );
      renderHook(() => useCurrentUser(true), { wrapper });
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 10));
      });
      mockGetCurrentUser.mockClear();

      const callOrder: string[] = [];

      mockLogin.mockImplementation(async () => {
        callOrder.push("login");
        return {
          access_token: "test-token",
          refresh_token: "test-refresh",
          token_type: "Bearer",
          expires_in: 900,
        };
      });

      mockGetCurrentUser.mockImplementation(async () => {
        // Simulate a delay in fetching user
        await new Promise((resolve) => setTimeout(resolve, 50));
        callOrder.push("getCurrentUser");
        return {
          id: "user-1",
          email: "test@example.com",
          name: "Test User",
          roles: ["user"],
        };
      });

      const { result } = renderHook(() => useLogin(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync({
          email: "test@example.com",
          password: "password123",
        });
        callOrder.push("mutation-resolved");
      });

      // Verify the order: login -> getCurrentUser -> mutation resolved
      expect(callOrder).toEqual(["login", "getCurrentUser", "mutation-resolved"]);
    });

    it("should update user cache after successful login", async () => {
      // Initialize the user query first
      await initializeUserQuery(null);

      const mockUser = {
        id: "user-1",
        email: "test@example.com",
        name: "Test User",
        roles: ["user"],
      };

      mockLogin.mockResolvedValueOnce({
        access_token: "test-token",
        refresh_token: "test-refresh",
        token_type: "Bearer",
        expires_in: 900,
      });
      mockGetCurrentUser.mockResolvedValueOnce(mockUser);

      const { result } = renderHook(() => useLogin(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync({
          email: "test@example.com",
          password: "password123",
        });
      });

      // Check that user data is in cache
      const cachedUser = queryClient.getQueryData(authKeys.user());
      expect(cachedUser).toEqual(mockUser);
    });
  });

  describe("useRegister", () => {
    it("should call register API and refetch user data on success", async () => {
      // Initialize the user query first
      await initializeUserQuery(null);

      const mockTokenResponse = {
        access_token: "test-token",
        refresh_token: "test-refresh",
        token_type: "Bearer",
        expires_in: 900,
      };
      const mockUser = {
        id: "user-1",
        email: "new@example.com",
        name: "New User",
        roles: ["user"],
      };

      mockRegister.mockResolvedValueOnce(mockTokenResponse);
      mockGetCurrentUser.mockResolvedValueOnce(mockUser);

      const { result } = renderHook(() => useRegister(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync({
          name: "New User",
          email: "new@example.com",
          password: "password123",
        });
      });

      expect(mockRegister).toHaveBeenCalledWith({
        name: "New User",
        email: "new@example.com",
        password: "password123",
      });
      expect(mockGetCurrentUser).toHaveBeenCalled();
    });

    it("should wait for user refetch to complete before mutation resolves", async () => {
      // Initialize the user query first
      mockGetCurrentUser.mockResolvedValueOnce(
        null as unknown as Awaited<ReturnType<typeof authApi.getCurrentUser>>,
      );
      renderHook(() => useCurrentUser(true), { wrapper });
      await act(async () => {
        await new Promise((resolve) => setTimeout(resolve, 10));
      });
      mockGetCurrentUser.mockClear();

      const callOrder: string[] = [];

      mockRegister.mockImplementation(async () => {
        callOrder.push("register");
        return {
          access_token: "test-token",
          refresh_token: "test-refresh",
          token_type: "Bearer",
          expires_in: 900,
        };
      });

      mockGetCurrentUser.mockImplementation(async () => {
        await new Promise((resolve) => setTimeout(resolve, 50));
        callOrder.push("getCurrentUser");
        return {
          id: "user-1",
          email: "new@example.com",
          name: "New User",
          roles: ["user"],
        };
      });

      const { result } = renderHook(() => useRegister(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync({
          name: "New User",
          email: "new@example.com",
          password: "password123",
        });
        callOrder.push("mutation-resolved");
      });

      // Verify the order: register -> getCurrentUser -> mutation resolved
      expect(callOrder).toEqual(["register", "getCurrentUser", "mutation-resolved"]);
    });

    it("should replace old user data with new user after registration", async () => {
      // Initialize the user query with old user (simulating previously logged in user)
      const oldUser = {
        id: "old-user",
        email: "old@example.com",
        name: "Old User",
        roles: ["admin"],
      };
      await initializeUserQuery(oldUser);

      const newUser = {
        id: "new-user",
        email: "new@example.com",
        name: "New User",
        roles: ["user"],
      };

      mockRegister.mockResolvedValueOnce({
        access_token: "test-token",
        refresh_token: "test-refresh",
        token_type: "Bearer",
        expires_in: 900,
      });
      mockGetCurrentUser.mockResolvedValueOnce(newUser);

      const { result } = renderHook(() => useRegister(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync({
          name: "New User",
          email: "new@example.com",
          password: "password123",
        });
      });

      // Verify old user is replaced with new user
      const cachedUser = queryClient.getQueryData(authKeys.user());
      expect(cachedUser).toEqual(newUser);
      expect(cachedUser).not.toEqual(oldUser);
    });
  });

  describe("useLogout", () => {
    it("should call logout API and clear all cached data", async () => {
      // Pre-populate cache with user data and org-sensitive data
      const mockUser = {
        id: "user-1",
        email: "test@example.com",
        name: "Test User",
        roles: ["user"],
      };
      queryClient.setQueryData(authKeys.user(), mockUser);
      queryClient.setQueryData(authKeys.personalAccessTokens(DEFAULT_ORG.public_id), [
        { id: "key-1", name: "Test Key" },
      ]);
      // Simulate org-sensitive cached data (e.g., durable workflows)
      queryClient.setQueryData(["durable", "workflows"], [{ id: "wf-1" }]);

      localStorage.setItem("everruns_current_org", "org_123");

      mockLogout.mockResolvedValueOnce();

      const { result } = renderHook(() => useLogout(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync();
      });

      expect(mockLogout).toHaveBeenCalled();

      // Verify ALL cached data is cleared, not just auth keys
      const cachedUser = queryClient.getQueryData(authKeys.user());
      expect(cachedUser).toBeUndefined();

      const cachedTokens = queryClient.getQueryData(
        authKeys.personalAccessTokens(DEFAULT_ORG.public_id),
      );
      expect(cachedTokens).toBeUndefined();

      // Verify org-sensitive data is also cleared
      const cachedWorkflows = queryClient.getQueryData(["durable", "workflows"]);
      expect(cachedWorkflows).toBeUndefined();

      // Verify localStorage org selection is cleared
      expect(localStorage.getItem("everruns_current_org")).toBeNull();
    });

    it("should clear user cache even if user was previously set", async () => {
      // Set initial user
      queryClient.setQueryData(authKeys.user(), {
        id: "user-1",
        email: "test@example.com",
        name: "Test User",
        roles: ["user"],
      });

      // Verify user exists before logout
      expect(queryClient.getQueryData(authKeys.user())).toBeDefined();

      mockLogout.mockResolvedValueOnce();

      const { result } = renderHook(() => useLogout(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync();
      });

      // Verify user is cleared after logout
      expect(queryClient.getQueryData(authKeys.user())).toBeUndefined();
    });
  });

  describe("org-scoped API key cache", () => {
    it("stores API key queries under the active org key and refetches after an org switch", async () => {
      mockListPersonalAccessTokens.mockResolvedValueOnce([
        {
          id: "key-default",
          name: "Default Key",
          token_prefix: "evr_default",
          scopes: ["*"],
          expires_at: undefined,
          last_used_at: undefined,
          created_at: "2024-01-01T00:00:00Z",
        },
      ]);

      const { result, rerender } = renderHook(() => usePersonalAccessTokens(), { wrapper });

      await waitFor(() => {
        expect(result.current.data).toEqual([
          expect.objectContaining({ id: "key-default", name: "Default Key" }),
        ]);
      });
      expect(
        queryClient.getQueryData(authKeys.personalAccessTokens(DEFAULT_ORG.public_id)),
      ).toEqual([expect.objectContaining({ id: "key-default", name: "Default Key" })]);

      mockUseOrg.mockReturnValue({
        currentOrg: SECOND_ORG,
        isLoading: false,
      });
      mockListPersonalAccessTokens.mockResolvedValueOnce([
        {
          id: "key-second",
          name: "Second Key",
          token_prefix: "evr_second",
          scopes: ["*"],
          expires_at: undefined,
          last_used_at: undefined,
          created_at: "2024-01-02T00:00:00Z",
        },
      ]);

      rerender();

      await waitFor(() => {
        expect(result.current.data).toEqual([
          expect.objectContaining({ id: "key-second", name: "Second Key" }),
        ]);
      });

      expect(
        queryClient.getQueryData(authKeys.personalAccessTokens(DEFAULT_ORG.public_id)),
      ).toEqual([expect.objectContaining({ id: "key-default", name: "Default Key" })]);
      expect(queryClient.getQueryData(authKeys.personalAccessTokens(SECOND_ORG.public_id))).toEqual(
        [expect.objectContaining({ id: "key-second", name: "Second Key" })],
      );
    });

    it("invalidates only the active org API key query after create", async () => {
      const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");
      mockCreatePersonalAccessToken.mockResolvedValueOnce({
        id: "token-created",
        name: "Created Token",
        token: "evr_pat_secret",
        token_prefix: "evr_pat_created",
        scopes: ["*"],
        expires_at: undefined,
        created_at: "2024-01-01T00:00:00Z",
      });

      const { result } = renderHook(() => useCreatePersonalAccessToken(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync({ name: "Created Key" });
      });

      expect(invalidateSpy).toHaveBeenCalledWith({
        queryKey: authKeys.personalAccessTokens(DEFAULT_ORG.public_id),
      });
    });

    it("invalidates only the active org API key query after delete", async () => {
      const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");
      mockDeletePersonalAccessToken.mockResolvedValueOnce(undefined);

      const { result } = renderHook(() => useDeletePersonalAccessToken(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync("key-delete");
      });

      expect(invalidateSpy).toHaveBeenCalledWith({
        queryKey: authKeys.personalAccessTokens(DEFAULT_ORG.public_id),
      });
    });
  });
});
