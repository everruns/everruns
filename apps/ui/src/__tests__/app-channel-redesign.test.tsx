import { fireEvent, render, screen } from "@testing-library/react";
import { CronLabel, describeCronExpression } from "@/components/apps/cron-label";
import {
  buildChannelConfig,
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
  it("renders cron labels as human-readable text with timezone", () => {
    render(<CronLabel expr="0 30 * * * * *" tz="America/Chicago" />);

    expect(screen.getByText("At 30 minutes past the hour · America/Chicago")).toBeInTheDocument();
    expect(screen.queryByText("0 30 * * * * *")).not.toBeInTheDocument();
  });

  it("builds backend-compatible schedule channel config", () => {
    const state = {
      ...getDefaultChannelFormState("schedule"),
      scheduleCronExpression: "0 30 * * * * *",
      scheduleTimezone: "America/Chicago",
      channelMessage: "Run {{app.name}} now.",
    };

    expect(isChannelFormValid(state)).toBe(true);
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

  it("rejects unsupported cron shapes before submit", () => {
    const base = {
      ...getDefaultChannelFormState("schedule"),
      channelMessage: "Run {{app.name}} now.",
    };

    expect(isChannelFormValid({ ...base, scheduleCronExpression: "0 */5 * * * *" })).toBe(false);
    expect(isChannelFormValid({ ...base, scheduleCronExpression: "0 0 9 * * * 2027" })).toBe(false);
    expect(isChannelFormValid({ ...base, scheduleCronExpression: "0 0 9 * * * *" })).toBe(true);
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
});
