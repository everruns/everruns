import { buildSlackChannelConfig } from "@/components/agents/channels/slack-form-config";
import { fireEvent, render, screen } from "@testing-library/react";
import { SlackConversationSettings } from "@/components/agents/channels/slack-conversation-settings";
import {
  getDefaultChannelFormState,
  buildChannelConfig,
} from "@/components/agents/channels/channel-form";

describe("Slack response policy", () => {
  const onChange = jest.fn();
  const settings = () => (
    <SlackConversationSettings
      sessionStrategy="per_thread"
      responsePolicy="all_messages"
      onResponsePolicyChange={onChange}
      onSessionStrategyChange={jest.fn()}
    />
  );

  it("lets a builder select relevant messages", async () => {
    render(settings());
    fireEvent.click(screen.getByLabelText("Response policy"));
    const option = await screen.findByRole("option", {
      name: "Relevant messages",
    });
    fireEvent.pointerDown(option);
    fireEvent.click(option);
    expect(onChange).toHaveBeenCalledWith("relevant_messages");
  });

  it("preserves the policy across endpoint editing and serialization", () => {
    const state = getDefaultChannelFormState("slack", {
      channel_type: "slack",
      channel_config: {
        response_policy: "relevant_messages",
        session_strategy: "per_thread",
      },
    } as Parameters<typeof getDefaultChannelFormState>[1]);
    expect(state.slackResponsePolicy).toBe("relevant_messages");
    expect(buildChannelConfig(state)).toMatchObject({
      response_policy: "relevant_messages",
    });
  });

  it("starts new channels per channel with relevant messages", () => {
    const fresh = getDefaultChannelFormState("slack");
    expect(fresh.slackSessionStrategy).toBe("per_channel");
    expect(fresh.slackResponsePolicy).toBe("relevant_messages");
    expect(buildSlackChannelConfig(fresh)).toMatchObject({
      session_strategy: "per_channel",
      response_policy: "relevant_messages",
    });
  });

  it("keeps the legacy defaults when editing a channel that never set them", () => {
    const state = getDefaultChannelFormState("slack", {
      channel_type: "slack",
      channel_config: {},
    } as Parameters<typeof getDefaultChannelFormState>[1]);
    expect(state.slackSessionStrategy).toBe("per_thread");
    expect(state.slackResponsePolicy).toBe("all_messages");
  });

  it("sends an explicit All messages choice so the server keeps it", () => {
    const state = {
      ...getDefaultChannelFormState("slack"),
      slackResponsePolicy: "all_messages",
    };
    expect(
      buildSlackChannelConfig(state as ReturnType<typeof getDefaultChannelFormState>)
        .response_policy,
    ).toBe("all_messages");
  });
});

it("roundtrips private approval visibility and allows explicit disabling", () => {
  const channel = {
    channel_type: "slack",
    channel_config: { private_approvals: true },
  } as Parameters<typeof getDefaultChannelFormState>[1];
  const state = getDefaultChannelFormState("slack", channel);
  expect(getDefaultChannelFormState("slack").slackPrivateApprovals).toBe(false);
  expect(state.slackPrivateApprovals).toBe(true);
  expect(buildSlackChannelConfig(state).private_approvals).toBe(true);
  expect(
    buildSlackChannelConfig({ ...state, slackPrivateApprovals: false }).private_approvals,
  ).toBe(false);
});
