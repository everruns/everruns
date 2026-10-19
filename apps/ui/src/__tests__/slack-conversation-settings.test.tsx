import { render, screen } from "@testing-library/react";
import { SlackConversationSettings } from "@/components/apps/slack-conversation-settings";

// Reply behavior moved to the agent-level Communication setting; Slack channels no longer
// carry a per-channel reply mode.
it("has no per-channel reply mode and points to the agent's Communication setting", () => {
  render(
    <SlackConversationSettings
      sessionStrategy="per_thread"
      responsePolicy="all_messages"
      onResponsePolicyChange={jest.fn()}
      onSessionStrategyChange={jest.fn()}
    />,
  );
  expect(screen.getByLabelText("Session strategy")).toBeInTheDocument();
  expect(screen.getByLabelText("Response policy")).toBeInTheDocument();
  expect(screen.queryByLabelText("Reply mode")).not.toBeInTheDocument();
  expect(screen.getByText(/set by Communication in its agent settings/)).toBeVisible();
  expect(
    screen.queryByText(/report_progress|tool_only|channel_post_message/),
  ).not.toBeInTheDocument();
});
