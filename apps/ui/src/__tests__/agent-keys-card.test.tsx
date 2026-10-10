import { fireEvent, render, screen, within } from "@testing-library/react";
import { AgentKeysCard } from "@/components/agents/channels/agent-keys-card";
import {
  buildChannelConfig,
  ChannelTypePicker,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/agents/channels/channel-form";
import type { AgentChannel, AgentKey } from "@/lib/api/types";

const createMutate = jest.fn();
const rotateMutate = jest.fn();
const revokeMutate = jest.fn();
let keys: AgentKey[] = [];

const mutation = (mutate: jest.Mock) => ({
  mutate,
  reset: jest.fn(),
  isPending: false,
  error: null,
});

jest.mock("@/hooks/use-agent-channels", () => ({
  useAgentKeys: () => ({ data: keys, isLoading: false, error: null }),
  useCreateAgentKey: () => mutation(createMutate),
  useRotateAgentKey: () => mutation(rotateMutate),
  useRevokeAgentKey: () => mutation(revokeMutate),
}));

jest.mock("@/providers/feature-flags-provider", () => ({
  useFeatureFlag: () => true,
}));

const key = (overrides: Partial<AgentKey> = {}): AgentKey => ({
  id: "agentkey_1",
  channel_id: "appchan_1",
  name: "Support backend",
  prefix: "evr_ak_1a2b3c4d...",
  permissions: ["sessions"],
  created_at: "2026-10-10T00:00:00Z",
  ...overrides,
});

describe("AgentKeysCard", () => {
  beforeEach(() => {
    jest.clearAllMocks();
    keys = [];
  });

  it("lists keys by prefix and offers rotate and revoke only for live keys", () => {
    keys = [key(), key({ id: "agentkey_2", name: "Old app", revoked_at: "2026-10-09T00:00:00Z" })];
    render(<AgentKeysCard agentId="agent_1" channelId="appchan_1" canManage />);
    const list = screen.getByRole("list", { name: "Agent keys" });
    const rows = within(list).getAllByRole("listitem");
    expect(rows).toHaveLength(2);
    expect(within(rows[0]).getByText("evr_ak_1a2b3c4d...")).toBeInTheDocument();
    expect(within(rows[0]).getByText("Never used")).toBeInTheDocument();
    expect(within(rows[0]).getByRole("button", { name: /Rotate/ })).toBeEnabled();
    expect(within(rows[1]).getByText("Revoked")).toBeInTheDocument();
    expect(within(rows[1]).queryByRole("button", { name: /Rotate/ })).not.toBeInTheDocument();
  });

  it("shows the secret once after creating a key", () => {
    createMutate.mockImplementation((_name, options) =>
      options.onSuccess({ ...key(), secret: "evr_ak_secretvalue" }),
    );
    render(<AgentKeysCard agentId="agent_1" channelId="appchan_1" canManage />);
    fireEvent.click(screen.getByRole("button", { name: /Create key/ }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "  Support backend " } });
    fireEvent.click(within(screen.getByRole("dialog")).getByRole("button", { name: "Create key" }));

    expect(createMutate).toHaveBeenCalledWith("Support backend", expect.anything());
    expect(screen.getByTestId("agent-key-secret")).toHaveTextContent("evr_ak_secretvalue");
    fireEvent.click(screen.getByRole("button", { name: "I stored it" }));
    expect(screen.queryByTestId("agent-key-secret")).not.toBeInTheDocument();
  });

  it("does not submit an enclosing form when the name form submits", () => {
    const outerSubmit = jest.fn((event) => event.preventDefault());
    render(
      <form onSubmit={outerSubmit}>
        <AgentKeysCard agentId="agent_1" channelId="appchan_1" canManage />
      </form>,
    );
    fireEvent.click(screen.getByRole("button", { name: /Create key/ }));
    fireEvent.change(screen.getByLabelText("Name"), { target: { value: "App" } });
    fireEvent.submit(screen.getByLabelText("Name"));
    expect(createMutate).toHaveBeenCalled();
    expect(outerSubmit).not.toHaveBeenCalled();
  });

  it("rotates with the chosen overlap and rejects an overlap past a week", () => {
    keys = [key()];
    render(<AgentKeysCard agentId="agent_1" channelId="appchan_1" canManage />);
    fireEvent.click(screen.getByRole("button", { name: /Rotate/ }));
    const overlap = screen.getByLabelText("Overlap (hours)");
    fireEvent.change(overlap, { target: { value: "169" } });
    expect(screen.getByRole("button", { name: "Rotate key" })).toBeDisabled();
    fireEvent.change(overlap, { target: { value: "0" } });
    fireEvent.click(screen.getByRole("button", { name: "Rotate key" }));
    expect(rotateMutate).toHaveBeenCalledWith(
      { keyId: "agentkey_1", overlapHours: 0 },
      expect.anything(),
    );
  });

  it("revokes after confirmation", () => {
    keys = [key()];
    render(<AgentKeysCard agentId="agent_1" channelId="appchan_1" canManage />);
    fireEvent.click(screen.getByRole("button", { name: /Revoke/ }));
    fireEvent.click(screen.getByRole("button", { name: "Revoke key" }));
    expect(revokeMutate).toHaveBeenCalledWith("agentkey_1", expect.anything());
  });

  it("disables key actions without manage permission", () => {
    keys = [key()];
    render(<AgentKeysCard agentId="agent_1" channelId="appchan_1" canManage={false} />);
    expect(screen.getByRole("button", { name: /Create key/ })).toBeDisabled();
    expect(screen.getByRole("button", { name: /Revoke/ })).toBeDisabled();
  });
});

describe("api channel form", () => {
  it("is offered in the channel type picker", () => {
    render(<ChannelTypePicker value="webhook" onChange={jest.fn()} />);
    expect(screen.getByText("Agent API")).toBeInTheDocument();
  });

  it("builds the default config with the server's defaults", () => {
    const state = getDefaultChannelFormState("api");
    expect(isChannelFormValid(state)).toBe(true);
    expect(buildChannelConfig(state)).toEqual({
      session_binding: "per_user",
      visibility: "activity",
      errors: "public",
      tool_approvals: "operator",
    });
  });

  it("round-trips a saved api channel and validates the rate limit", () => {
    const channel = {
      channel_type: "api",
      enabled: true,
      channel_config: {
        session_binding: "session_per_invocation",
        visibility: "full",
        errors: "detailed",
        tool_approvals: "caller",
        tool_activity_text: "Looking it up",
        rate_limit_per_minute: 60,
      },
    } as unknown as AgentChannel;
    const state = getDefaultChannelFormState("api", channel);
    expect(state.kind).toBe("api");
    expect(buildChannelConfig(state)).toEqual(channel.channel_config);

    const bad = { ...state, api: { ...state.api, rateLimitPerMinute: "1000001" } };
    expect(isChannelFormValid(bad)).toBe(false);
    const cleared = { ...state, api: { ...state.api, rateLimitPerMinute: " " } };
    expect(buildChannelConfig(cleared)).not.toHaveProperty("rate_limit_per_minute");
  });

  it("keeps identity providers and validates browser origins", () => {
    const authMethods = [{ mode: "oidc", provider: { type: "oidc", issuer: "https://idp" } }];
    const channel = {
      channel_type: "api",
      enabled: true,
      channel_config: {
        auth_methods: authMethods,
        cors_origins: ["https://app.example.com", "http://localhost:3000"],
      },
    } as unknown as AgentChannel;
    const state = getDefaultChannelFormState("api", channel);
    expect(buildChannelConfig(state)).toMatchObject({
      auth_methods: authMethods,
      cors_origins: ["https://app.example.com", "http://localhost:3000"],
    });
    for (const bad of ["https://app.example.com/", "http://app.example.com", "*"]) {
      const invalid = { ...state, api: { ...state.api, corsOrigins: bad } };
      expect(isChannelFormValid(invalid)).toBe(false);
    }
  });
});
