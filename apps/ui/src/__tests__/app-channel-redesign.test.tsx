import { fireEvent, render, screen } from "@testing-library/react";
import { CronLabel, describeCronExpression } from "@/components/apps/cron-label";
import {
  buildChannelConfig,
  CHANNEL_FORM_KINDS,
  ChannelForm,
  getDefaultChannelFormState,
  isChannelFormValid,
} from "@/components/apps/channel-form";
import { ChannelRow } from "@/components/apps/channel-row";
import type { App, AppChannel } from "@/lib/api/types";

const app: App = {
  id: "app_123",
  name: "Dad Joke Hourly",
  description: "Runs hourly.",
  harness_id: "harness_123",
  agent_id: null,
  agent_version_policy: "default",
  agent_version_id: null,
  owner_principal_id: "principal_123",
  channels: [],
  status: "published",
  published_at: "2026-05-10T00:00:00Z",
  created_at: "2026-05-10T00:00:00Z",
  updated_at: "2026-05-10T00:00:00Z",
  archived_at: null,
  deleted_at: null,
};

const scheduleChannel: AppChannel = {
  id: "appchan_123",
  channel_type: "schedule",
  channel_config: {
    cron_expression: "0 30 * * * * *",
    timezone: "America/Chicago",
    session_mode: "shared_session",
    message: "Tell a dad joke for {{app.name}}.",
  },
  enabled: true,
  created_at: "2026-05-10T00:00:00Z",
  updated_at: "2026-05-10T00:00:00Z",
};

describe("app channel redesign", () => {
  it("generates a token by default for new AG-UI channels", () => {
    const state = getDefaultChannelFormState("ag_ui");
    const config = buildChannelConfig(state);

    expect(state.agUiToken).toMatch(/^[A-Za-z0-9\-_]{16,}$/);
    expect(config).toEqual(expect.objectContaining({ anonymous: true, token: state.agUiToken }));
  });

  it("preserves AG-UI endpoint auth and anonymous settings when editing", () => {
    const preservedAuth = {
      mode: "google_oidc",
      provider: { type: "google_oidc", client_id: "client-123" },
      requirements: { domains: ["example.com"] },
    } as const;
    const channel: AppChannel = {
      id: "appchan_agui",
      channel_type: "ag_ui",
      channel_config: {
        anonymous: false,
        session_expiration_seconds: 3600,
        tool_visibility: "narrated",
        auth: preservedAuth,
      },
      enabled: true,
      created_at: "2026-05-10T00:00:00Z",
      updated_at: "2026-05-10T00:00:00Z",
    };

    const state = getDefaultChannelFormState("ag_ui", channel);

    expect(buildChannelConfig(state)).toEqual(
      expect.objectContaining({
        anonymous: false,
        auth: preservedAuth,
      }),
    );
  });

  it("renders cron labels as human-readable text with timezone", () => {
    render(<CronLabel expr="0 30 * * * * *" tz="America/Chicago" />);

    expect(screen.getByText("At 30 minutes past the hour · America/Chicago")).toBeInTheDocument();
    expect(screen.queryByText("0 30 * * * * *")).not.toBeInTheDocument();
  });

  it("keeps legacy schedule config readable but rejects new schedule channels", () => {
    const state = {
      ...getDefaultChannelFormState("schedule"),
      scheduleCronExpression: "0 30 * * * * *",
      scheduleTimezone: "America/Chicago",
      channelMessage: "Run {{app.name}} now.",
    };

    expect(CHANNEL_FORM_KINDS).not.toContain("schedule");
    expect(isChannelFormValid(state)).toBe(false);
    expect(buildChannelConfig(state)).toEqual({
      cron_expression: "0 30 * * * * *",
      timezone: "America/Chicago",
      session_mode: "shared_session",
      message: "Run {{app.name}} now.",
    });
    expect(describeCronExpression(state.scheduleCronExpression)).toBe(
      "At 30 minutes past the hour",
    );
  });

  it("rejects schedule channels before submit regardless of cron shape", () => {
    const base = {
      ...getDefaultChannelFormState("schedule"),
      channelMessage: "Run {{app.name}} now.",
    };

    expect(isChannelFormValid({ ...base, scheduleCronExpression: "0 */5 * * * *" })).toBe(false);
    expect(isChannelFormValid({ ...base, scheduleCronExpression: "0 0 9 * * * 2027" })).toBe(false);
    expect(isChannelFormValid({ ...base, scheduleCronExpression: "0 0 9 * * * *" })).toBe(false);
  });

  it("shows raw cron only inside the editable cron input", () => {
    const state = {
      ...getDefaultChannelFormState("schedule"),
      scheduleCronExpression: "0 30 * * * * *",
      scheduleTimezone: "America/Chicago",
      channelMessage: "Run {{app.name}} now.",
    };

    render(<ChannelForm state={state} onChange={() => {}} mode="new" section="schedule" />);

    expect(screen.getByLabelText("Cron expression")).toHaveValue("0 30 * * * * *");
    expect(screen.getByText("At 30 minutes past the hour")).toBeInTheDocument();
  });

  it("updates channel form state when cron preset is selected", () => {
    const onChange = jest.fn();
    const state = {
      ...getDefaultChannelFormState("schedule"),
      channelMessage: "Run {{app.name}} now.",
    };

    render(<ChannelForm state={state} onChange={onChange} mode="new" section="schedule" />);

    fireEvent.click(screen.getByRole("button", { name: "Hourly :30" }));

    expect(onChange).toHaveBeenCalledWith(
      expect.objectContaining({ scheduleCronExpression: "0 30 * * * * *" }),
    );
  });

  it("renders schedule channel rows without exposing raw cron", () => {
    render(
      <ChannelRow
        app={{ ...app, channels: [scheduleChannel] }}
        channel={scheduleChannel}
        expanded={false}
        onToggle={() => {}}
        configureHref="/apps/app_123/channels/appchan_123"
      />,
    );

    expect(screen.getByText("At 30 minutes past the hour · America/Chicago")).toBeInTheDocument();
    expect(screen.queryByText("0 30 * * * * *")).not.toBeInTheDocument();
    expect(screen.getByText(/Active$/)).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Tell a dad joke/ })).toBeInTheDocument();
  });

  it("disables Run now when onRunNow is not provided (no manage permission)", async () => {
    const { getByRole } = render(
      <ChannelRow
        app={{ ...app, channels: [scheduleChannel] }}
        channel={scheduleChannel}
        expanded={false}
        onToggle={() => {}}
        configureHref="/apps/app_123/channels/appchan_123"
        // no onRunNow — simulates caller withholding the action due to !canManage
      />,
    );

    const trigger = getByRole("button", { name: "Channel actions" });
    fireEvent.click(trigger);

    const runNow = await screen.findByText("Run now");
    expect(
      runNow.closest("[aria-disabled]") ?? runNow.closest("[disabled]") ?? runNow,
    ).toHaveAttribute("data-disabled");
  });

  it("enables Run now when onRunNow is provided and channel is runnable", async () => {
    const { getByRole } = render(
      <ChannelRow
        app={{ ...app, channels: [scheduleChannel] }}
        channel={scheduleChannel}
        expanded={false}
        onToggle={() => {}}
        onRunNow={() => {}}
        configureHref="/apps/app_123/channels/appchan_123"
      />,
    );

    const trigger = getByRole("button", { name: "Channel actions" });
    fireEvent.click(trigger);

    const runNow = await screen.findByText("Run now");
    expect(runNow.closest("[data-disabled]")).toBeNull();
  });
});
