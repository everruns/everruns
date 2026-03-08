import { renderHook, act } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { ReactNode } from "react";
import { usePublishApp, useUnpublishApp, useUpdateApp } from "@/hooks/use-apps";
import { queryKeys } from "@/lib/query-keys";
import type { App, UpdateAppRequest } from "@/lib/api/types";

jest.mock("@/lib/api/apps", () => ({
  createApp: jest.fn(),
  deleteApp: jest.fn(),
  getApp: jest.fn(),
  getApps: jest.fn(),
  publishApp: jest.fn(),
  unpublishApp: jest.fn(),
  updateApp: jest.fn(),
}));

const mockUseOrg = jest.fn();
jest.mock("@/providers/org-provider", () => ({
  useOrg: () => mockUseOrg(),
}));

import * as appsApi from "@/lib/api/apps";

const mockPublishApp = appsApi.publishApp as jest.MockedFunction<typeof appsApi.publishApp>;
const mockUnpublishApp = appsApi.unpublishApp as jest.MockedFunction<typeof appsApi.unpublishApp>;
const mockUpdateApp = appsApi.updateApp as jest.MockedFunction<typeof appsApi.updateApp>;

function makeApp(overrides: Partial<App> = {}): App {
  return {
    id: "app-123",
    name: "Test App",
    description: "Slack bot",
    harness_id: "hrs-123",
    agent_id: "agt-123",
    channel_type: "slack",
    channel_config: {
      signing_secret: "secret",
      bot_token: "xoxb-test",
      session_strategy: "per_thread",
    },
    status: "draft",
    published_at: null,
    created_at: "2026-03-08T22:23:38Z",
    updated_at: "2026-03-08T22:24:23Z",
    ...overrides,
  };
}

describe("use-apps mutations", () => {
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

    mockUseOrg.mockReturnValue({
      currentOrg: { public_id: "org-123" },
      isLoading: false,
    });
  });

  it.each([
    {
      name: "update",
      useHook: useUpdateApp,
      mutationFn: mockUpdateApp,
      args: { appId: "app-123", data: { name: "Updated App" } satisfies UpdateAppRequest },
      response: makeApp({ name: "Updated App" }),
    },
    {
      name: "publish",
      useHook: usePublishApp,
      mutationFn: mockPublishApp,
      args: "app-123",
      response: makeApp({
        status: "published",
        published_at: "2026-03-08T22:30:00Z",
      }),
    },
    {
      name: "unpublish",
      useHook: useUnpublishApp,
      mutationFn: mockUnpublishApp,
      args: "app-123",
      response: makeApp({
        status: "draft",
        published_at: null,
      }),
    },
  ])("invalidates list and detail queries after $name succeeds", async (testCase) => {
    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");
    const existingApp = makeApp();
    queryClient.setQueryData(queryKeys.apps.detail(existingApp.id), existingApp);
    queryClient.setQueryData(queryKeys.apps.list(), [existingApp]);
    testCase.mutationFn.mockResolvedValueOnce(testCase.response);

    const { result } = renderHook(() => testCase.useHook(), { wrapper });

    await act(async () => {
      await result.current.mutateAsync(testCase.args as never);
    });

    expect(queryClient.getQueryData(queryKeys.apps.detail(testCase.response.id))).toEqual(
      testCase.response,
    );
    expect(queryClient.getQueryData(queryKeys.apps.list())).toEqual([testCase.response]);
    expect(invalidateSpy).toHaveBeenCalledWith({ queryKey: queryKeys.apps.all });
    expect(invalidateSpy).toHaveBeenCalledWith({
      queryKey: queryKeys.apps.detail(testCase.response.id),
    });
  });

  it("does not invalidate app queries when publish fails", async () => {
    const invalidateSpy = jest.spyOn(queryClient, "invalidateQueries");
    mockPublishApp.mockRejectedValueOnce(new Error("publish failed"));

    const { result } = renderHook(() => usePublishApp(), { wrapper });

    await act(async () => {
      await expect(result.current.mutateAsync("app-123")).rejects.toThrow("publish failed");
    });

    expect(invalidateSpy).not.toHaveBeenCalled();
  });
});
