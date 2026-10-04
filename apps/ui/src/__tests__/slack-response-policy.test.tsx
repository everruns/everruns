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
      replyMode="all_messages"
      responsePolicy="all_messages"
      onResponsePolicyChange={onChange}
      onReplyModeChange={jest.fn()}
      onSessionStrategyChange={jest.fn()}
    />
  );

  it("offers participation settings without feature enrollment", () => {
    render(settings());
    expect(screen.getByLabelText("Response policy")).toBeInTheDocument();
    expect(screen.getByLabelText("Reply mode")).toBeInTheDocument();
  });

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
