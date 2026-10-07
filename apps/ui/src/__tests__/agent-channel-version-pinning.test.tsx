import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { Suspense } from "react";
import AgentTriggerPage from "@/app/(main)/agents/[agentId]/triggers/[triggerId]/page";
import { AgentChannelEditor } from "@/components/agents/agent-channel-editor";
import type {
  Agent,
  AgentTrigger,
  AgentVersion,
  AgentChannel,
  OpenApiAgentChannel,
} from "@/lib/api/types";

const update = jest.fn();
const deleteChannel = jest.fn();
let mockDeleteError: Error | null = null;
const updateTrigger = jest.fn().mockResolvedValue({});
let mockTrigger: AgentTrigger;
let mockChannel: AgentChannel;
let mockFlagEnabled = true;

const savedVersion = {
  id: "agentver_saved",
  version: "0.1.0",
  summary: "Initial",
  is_published: true,
} as AgentVersion;
const newerVersion = {
  id: "agentver_newer",
  version: "0.2.0",
  summary: null,
  is_published: true,
} as AgentVersion;
const draftSnapshot = {
  id: "agentver_draft",
  version: "draft.3",
  summary: null,
  is_published: false,
} as AgentVersion;

jest.mock("next/navigation", () => ({
  useRouter: () => ({ push: jest.fn(), replace: jest.fn() }),
  usePathname: () => "/agents/agent_123/channels/appchan_123",
  useSearchParams: () => new URLSearchParams(),
}));

jest.mock("@/hooks/use-agent-triggers", () => ({
  useAgentTriggers: () => ({ data: [mockTrigger], isLoading: false }),
  useCreateAgentTrigger: () => ({ mutateAsync: jest.fn(), isPending: false }),
  useUpdateAgentTrigger: () => ({ mutateAsync: updateTrigger, isPending: false }),
  useDeleteAgentTrigger: () => ({ mutate: jest.fn(), isPending: false }),
  useRunAgentTrigger: () => ({ mutate: jest.fn(), isPending: false }),
}));

jest.mock("@/components/agents/trigger-form", () => ({
  EMPTY_TRIGGER_FORM: {
    cron_expression: "0 0 9 * * * *",
    timezone: "UTC",
    session_mode: "shared_session",
    message: "",
    enabled: true,
  },
  isTriggerFormValid: () => true,
  TriggerFormFields: () => <div data-testid="trigger-form" />,
}));

jest.mock("next/link", () => ({
  __esModule: true,
  default: ({ children, href, ...props }: React.ComponentPropsWithoutRef<"a">) => (
    <a href={href} {...props}>
      {children}
    </a>
  ),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: (flag: string) => flag === "agent_versions" && mockFlagEnabled,
}));

jest.mock("@/hooks/use-agents", () => ({
  useAgent: () => ({
    data: { id: "agent_123", name: "support", status: "active" } as Agent,
    isLoading: false,
  }),
  useAgentVersions: () => ({
    data: [newerVersion, draftSnapshot, savedVersion],
    isLoading: false,
  }),
}));

jest.mock("@/hooks/use-policies", () => ({
  usePolicies: () => ({ can: () => true, isLoading: false }),
}));

jest.mock("@/components/health/channel-health-warning", () => ({
  ChannelHealthWarning: () => null,
}));

jest.mock("@/hooks/use-agent-channels", () => ({
  useAgentChannels: () => ({ channels: [{ channel: mockChannel }], isLoading: false }),
  useSlackInstallCapability: () => ({ data: undefined, isLoading: false, refetch: jest.fn() }),
  useUpdateAgentChannel: () => ({ mutate: update, isPending: false }),
  useDeleteAgentChannel: () => ({
    mutate: deleteChannel,
    isPending: false,
    error: mockDeleteError,
  }),
  usePublishAgentChannel: () => ({ mutate: jest.fn(), isPending: false }),
  useTriggerAgentChannel: () => ({ mutate: jest.fn(), isPending: false }),
}));

// The transport form is covered elsewhere; this suite is about the version
// selection the editor sends alongside it.
jest.mock("@/components/agents/channels/channel-form", () => ({
  ChannelForm: () => <div data-testid="channel-form" />,
  getDefaultChannelFormState: () => ({ enabled: true }),
  buildChannelConfig: () => ({ message: "Process {{payload}}" }),
  isChannelFormValid: () => true,
}));

// Radix Select does not open in jsdom; a native select keeps the same
// label/value/onValueChange contract.
jest.mock("@/components/ui/select", () => {
  const React = jest.requireActual<typeof import("react")>("react");
  function SelectTrigger(_: { id?: string; children?: React.ReactNode }) {
    return null;
  }
  function SelectContent(_: { children?: React.ReactNode }) {
    return null;
  }
  function Select({
    value,
    onValueChange,
    disabled,
    children,
  }: {
    value?: string;
    onValueChange: (value: string) => void;
    disabled?: boolean;
    children: React.ReactNode;
  }) {
    let id: string | undefined;
    let items: React.ReactNode = null;
    React.Children.forEach(children, (child) => {
      if (!React.isValidElement(child)) return;
      const props = child.props as { id?: string; children?: React.ReactNode };
      if (child.type === SelectTrigger) id = props.id;
      if (child.type === SelectContent) items = props.children;
    });
    return (
      <select
        id={id}
        value={value ?? ""}
        disabled={disabled}
        onChange={(event) => onValueChange(event.target.value)}
      >
        <option value="" disabled>
          none
        </option>
        {items}
      </select>
    );
  }
  return {
    Select,
    SelectTrigger,
    SelectContent,
    SelectValue: () => null,
    SelectItem: ({
      value,
      disabled,
      children,
    }: {
      value: string;
      disabled?: boolean;
      children: React.ReactNode;
    }) => (
      <option value={value} disabled={disabled}>
        {children}
      </option>
    ),
  };
});

function channel(
  overrides: Partial<Pick<OpenApiAgentChannel, "agent_version_policy" | "agent_version_id">> = {},
): AgentChannel {
  return {
    id: "appchan_123",
    channel_type: "webhook",
    channel_config: { message: "Process {{payload}}" },
    enabled: true,
    status: "live",
    created_at: "2026-10-01T00:00:00Z",
    updated_at: "2026-10-01T00:00:00Z",
    agent_version_policy: "default",
    ...overrides,
  } as AgentChannel;
}

function renderEditor() {
  render(<AgentChannelEditor agentId="agent_123" channelId="appchan_123" />);
}

function save() {
  fireEvent.click(screen.getByRole("button", { name: "Save" }));
  return update.mock.calls[0][0] as Record<string, unknown>;
}

describe("Agent channel version pinning", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockFlagEnabled = true;
    mockChannel = channel();
    mockDeleteError = null;
  });

  it("confirms Slack app removal before deleting the channel and keeps failures visible", async () => {
    mockChannel = {
      ...channel(),
      channel_type: "slack",
      channel_config: { slack_app_provisioned: true },
    };
    mockDeleteError = new Error("Could not remove the agent from Slack. Retry.");
    renderEditor();
    fireEvent.click(screen.getByRole("button", { name: "Delete" }));
    expect(deleteChannel).not.toHaveBeenCalled();
    expect(await screen.findByRole("dialog")).toHaveTextContent("removed from Slack");
    expect(screen.getByRole("dialog")).toHaveTextContent("Could not remove the agent from Slack");
    fireEvent.click(screen.getByRole("button", { name: "Delete channel" }));
    expect(deleteChannel).toHaveBeenCalledTimes(1);
  });

  it("pins a saved version, offering only saved versions", () => {
    renderEditor();

    fireEvent.change(screen.getByLabelText("Runs"), { target: { value: "pinned" } });
    const versionPicker = screen.getByLabelText("Pinned version") as HTMLSelectElement;
    const offered = Array.from(versionPicker.options)
      .map((option) => option.value)
      .filter(Boolean);
    expect(offered).toEqual(["agentver_newer", "agentver_saved"]);

    fireEvent.change(versionPicker, { target: { value: "agentver_saved" } });
    expect(save()).toEqual(
      expect.objectContaining({
        agent_version_policy: "pinned",
        agent_version_id: "agentver_saved",
      }),
    );
  });

  it("shows an existing pin and unpins it back to the default", () => {
    mockChannel = channel({
      agent_version_policy: "pinned",
      agent_version_id: "agentver_saved",
    });
    renderEditor();

    expect(screen.getByText("Pinned to 0.1.0")).toBeInTheDocument();
    expect((screen.getByLabelText("Pinned version") as HTMLSelectElement).value).toBe(
      "agentver_saved",
    );

    fireEvent.change(screen.getByLabelText("Runs"), { target: { value: "default" } });
    expect(screen.queryByLabelText("Pinned version")).not.toBeInTheDocument();
    const request = save();
    expect(request.agent_version_policy).toBe("default");
    expect(request).not.toHaveProperty("agent_version_id");
  });

  it("does not resend an untouched selection with transport changes", () => {
    mockChannel = channel({
      agent_version_policy: "pinned",
      agent_version_id: "agentver_saved",
    });
    renderEditor();

    const request = save();
    expect(request).not.toHaveProperty("agent_version_policy");
    expect(request).not.toHaveProperty("agent_version_id");
  });

  it("keeps an App-era pin visible and removable when the feature is off", () => {
    mockFlagEnabled = false;
    mockChannel = channel({
      agent_version_policy: "pinned",
      agent_version_id: "agentver_saved",
    });
    renderEditor();

    const policy = screen.getByLabelText("Runs") as HTMLSelectElement;
    expect(policy).not.toBeDisabled();
    expect(
      Array.from(policy.options)
        .filter((option) => option.disabled && option.value)
        .map((option) => option.value),
    ).toEqual(["latest", "pinned"]);

    fireEvent.change(policy, { target: { value: "default" } });
    expect(save().agent_version_policy).toBe("default");
  });

  it("hides the control for unpinned channels when the feature is off", () => {
    mockFlagEnabled = false;
    renderEditor();
    expect(screen.queryByLabelText("Runs")).not.toBeInTheDocument();
  });
});

describe("Agent trigger version pinning", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    mockFlagEnabled = true;
    mockTrigger = {
      id: "trg_123",
      agent_id: "agent_123",
      trigger_type: "schedule",
      config: {
        cron_expression: "0 0 9 * * * *",
        timezone: "UTC",
        session_mode: "shared_session",
        message: "Daily digest",
      },
      enabled: true,
      agent_version_policy: "default",
      created_at: "2026-10-01T00:00:00Z",
      updated_at: "2026-10-01T00:00:00Z",
    } as AgentTrigger;
  });

  async function renderTriggerPage() {
    const params = Promise.resolve({ agentId: "agent_123", triggerId: "trg_123" });
    await act(async () => {
      render(
        <Suspense fallback={<div>Loading...</div>}>
          <AgentTriggerPage params={params} />
        </Suspense>,
      );
      await params;
    });
  }

  it("pins a trigger to a saved version", async () => {
    await renderTriggerPage();

    fireEvent.change(screen.getByLabelText("Runs"), { target: { value: "pinned" } });
    fireEvent.change(screen.getByLabelText("Pinned version"), {
      target: { value: "agentver_saved" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(updateTrigger).toHaveBeenCalled());
    expect(updateTrigger.mock.calls[0][0].request).toEqual(
      expect.objectContaining({
        message: "Daily digest",
        agent_version_policy: "pinned",
        agent_version_id: "agentver_saved",
      }),
    );
  });

  it("unpins a pinned trigger", async () => {
    mockTrigger = {
      ...mockTrigger,
      agent_version_policy: "pinned",
      agent_version_id: "agentver_saved",
    };
    await renderTriggerPage();

    expect((screen.getByLabelText("Pinned version") as HTMLSelectElement).value).toBe(
      "agentver_saved",
    );
    fireEvent.change(screen.getByLabelText("Runs"), { target: { value: "default" } });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));

    await waitFor(() => expect(updateTrigger).toHaveBeenCalled());
    const request = updateTrigger.mock.calls[0][0].request;
    expect(request.agent_version_policy).toBe("default");
    expect(request).not.toHaveProperty("agent_version_id");
  });
});
