import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";

import { ChannelEditor } from "@/components/apps/channel-editor";
import type { App, AppChannel } from "@/lib/api/types";

const mockUseApp = jest.fn();
const mockPublishChannel = jest.fn();
const mockCan = jest.fn();

jest.mock("next/navigation", () => ({
  usePathname: () => "/agents/agent_123/endpoints/appchan_draft",
  useRouter: () => ({ push: jest.fn(), replace: jest.fn() }),
}));

jest.mock("@/hooks/use-apps", () => ({
  useApp: () => mockUseApp(),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: mockCan, isLoading: false }),
}));

jest.mock("@/lib/api/apps", () => ({
  deleteChannel: jest.fn(),
  publishChannel: (...args: unknown[]) => mockPublishChannel(...args),
  triggerChannel: jest.fn(),
  unpublishChannel: jest.fn(),
  updateChannel: jest.fn(),
}));

const draftChannel: AppChannel = {
  id: "appchan_draft",
  channel_type: "ag_ui",
  channel_config: {
    anonymous: true,
    token_configured: true,
    session_expiration_seconds: 3600,
    tool_visibility: "generic",
  },
  enabled: true,
  status: "draft",
  created_at: "2026-09-16T00:00:00Z",
  updated_at: "2026-09-16T00:00:00Z",
};

const publishedApp: App = {
  id: "app_published",
  name: "Published app",
  description: null,
  harness_id: "harness_123",
  agent_id: "agent_123",
  agent_version_policy: "default",
  agent_version_id: null,
  owner_principal_id: "principal_123",
  channels: [draftChannel],
  status: "published",
  published_at: "2026-09-16T00:00:00Z",
  created_at: "2026-09-16T00:00:00Z",
  updated_at: "2026-09-16T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

describe("ChannelEditor endpoint lifecycle", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockUseApp.mockReturnValue({ data: publishedApp, isLoading: false });
    mockPublishChannel.mockResolvedValue({ ...draftChannel, status: "live" });
    mockCan.mockReturnValue(true);
  });

  it("shows and publishes an enabled draft endpoint instead of calling it active", async () => {
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <ChannelEditor
          appId={publishedApp.id}
          channelId={draftChannel.id}
          nav={{ breadcrumbs: [], returnHref: "/agents/agent_123", returnLabel: "Test agent" }}
        />
      </QueryClientProvider>,
    );

    expect(await screen.findByText(/Draft — not accepting traffic$/)).toBeInTheDocument();
    expect(screen.getByText("draft")).toBeInTheDocument();
    expect(screen.queryByText("active")).not.toBeInTheDocument();

    fireEvent.click(screen.getByRole("button", { name: "Publish" }));

    await waitFor(() =>
      expect(mockPublishChannel).toHaveBeenCalledWith(publishedApp.id, draftChannel.id),
    );
  });

  it("does not publish when channel management is allowed without dangerous actions", async () => {
    mockCan.mockImplementation((action: string) => action === "app.manage");
    const queryClient = new QueryClient({
      defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
    });

    render(
      <QueryClientProvider client={queryClient}>
        <ChannelEditor
          appId={publishedApp.id}
          channelId={draftChannel.id}
          nav={{ breadcrumbs: [], returnHref: "/agents/agent_123", returnLabel: "Test agent" }}
        />
      </QueryClientProvider>,
    );

    const publishButton = await screen.findByRole("button", { name: "Publish" });
    expect(publishButton).toBeDisabled();

    fireEvent.click(publishButton);
    expect(mockPublishChannel).not.toHaveBeenCalled();
  });
});
