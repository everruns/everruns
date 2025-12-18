import { renderHook, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import {
  useLogin,
  useRegister,
  useLogout,
  useCurrentUser,
  authKeys,
} from "@/hooks/use-auth";

// Mock the API functions
jest.mock("@/lib/api/auth", () => ({
  login: jest.fn(),
  register: jest.fn(),
  logout: jest.fn(),
  getCurrentUser: jest.fn(),
  getAuthConfig: jest.fn(),
  listApiKeys: jest.fn(),
  createApiKey: jest.fn(),
  deleteApiKey: jest.fn(),
}));

import * as authApi from "@/lib/api/auth";

const mockLogin = authApi.login as jest.MockedFunction<typeof authApi.login>;
const mockRegister = authApi.register as jest.MockedFunction<typeof authApi.register>;
const mockLogout = authApi.logout as jest.MockedFunction<typeof authApi.logout>;
const mockGetCurrentUser = authApi.getCurrentUser as jest.MockedFunction<typeof authApi.getCurrentUser>;

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
  });

  // Helper to initialize the user query so refetchQueries has something to refetch
  async function initializeUserQuery(initialUser: unknown = null) {
    mockGetCurrentUser.mockResolvedValueOnce(initialUser as Awaited<ReturnType<typeof authApi.getCurrentUser>>);
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
      mockGetCurrentUser.mockResolvedValueOnce(null as unknown as Awaited<ReturnType<typeof authApi.getCurrentUser>>);
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
      mockGetCurrentUser.mockResolvedValueOnce(null as unknown as Awaited<ReturnType<typeof authApi.getCurrentUser>>);
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
    it("should call logout API and remove user data from cache", async () => {
      // Pre-populate cache with user data
      const mockUser = {
        id: "user-1",
        email: "test@example.com",
        name: "Test User",
        roles: ["user"],
      };
      queryClient.setQueryData(authKeys.user(), mockUser);
      queryClient.setQueryData(authKeys.apiKeys(), [{ id: "key-1", name: "Test Key" }]);

      mockLogout.mockResolvedValueOnce();

      const { result } = renderHook(() => useLogout(), { wrapper });

      await act(async () => {
        await result.current.mutateAsync();
      });

      expect(mockLogout).toHaveBeenCalled();

      // Verify user data is removed from cache
      const cachedUser = queryClient.getQueryData(authKeys.user());
      expect(cachedUser).toBeUndefined();

      // Verify API keys are also removed
      const cachedApiKeys = queryClient.getQueryData(authKeys.apiKeys());
      expect(cachedApiKeys).toBeUndefined();
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
});
