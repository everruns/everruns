import { buildSlackChannelConfig } from "@/components/agents/channels/slack-form-config";
import { fireEvent, render, screen } from "@testing-library/react";
import { SlackConversationSettings } from "@/components/apps/slack-conversation-settings";
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
    const option = await screen.findByRole("option", { name: "Relevant messages" });
    fireEvent.pointerDown(option);
    fireEvent.click(option);
    expect(onChange).toHaveBeenCalledWith("relevant_messages");
  });

  it("preserves the policy across endpoint editing and serialization", () => {
    const state = getDefaultChannelFormState("slack", {
      channel_type: "slack",
      channel_config: { response_policy: "relevant_messages", session_strategy: "per_thread" },
    } as Parameters<typeof getDefaultChannelFormState>[1]);
    expect(state.slackResponsePolicy).toBe("relevant_messages");
    expect(buildChannelConfig(state)).toMatchObject({ response_policy: "relevant_messages" });
    expect(getDefaultChannelFormState("slack").slackResponsePolicy).toBe("all_messages");
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
