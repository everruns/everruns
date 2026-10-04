import { fireEvent, render, screen } from "@testing-library/react";
import { useState } from "react";
import { SlackConversationSettings } from "@/components/apps/slack-conversation-settings";
import type { SlackReplyMode } from "@/lib/api/types";

function Settings({ onChange }: { onChange: (value: SlackReplyMode) => void }) {
  const [mode, setMode] = useState<SlackReplyMode>("all_messages");
  return (
    <SlackConversationSettings
      sessionStrategy="per_thread"
      replyMode={mode}
      responsePolicy="all_messages"
      onResponsePolicyChange={jest.fn()}
      onSessionStrategyChange={jest.fn()}
      onReplyModeChange={(value) => {
        setMode(value);
        onChange(value);
      }}
    />
  );
}

it("explains agent-controlled communication, working feedback, and saves the canonical mode", async () => {
  const onChange = jest.fn();
  render(<Settings onChange={onChange} />);
  expect(screen.getByLabelText("Reply mode")).toHaveTextContent("Automatic replies");
  expect(screen.getByText(/Automatically post every assistant response/)).toBeVisible();
  fireEvent.click(screen.getByLabelText("Reply mode"));
  const option = await screen.findByRole("option", { name: "Agent-controlled messages" });
  fireEvent.pointerDown(option, { pointerType: "mouse" });
  fireEvent.click(option);
  expect(onChange).toHaveBeenLastCalledWith("tool_only");
  expect(screen.getByLabelText("Reply mode")).toHaveTextContent("Agent-controlled messages");
  expect(
    screen.getByText(/The agent chooses when to send updates, questions, and answers/),
  ).toBeVisible();
  expect(screen.getByText(/Slack acknowledges each request while the agent works/)).toBeVisible();
  expect(
    screen.queryByText(/report_progress|tool_only|channel_post_message/),
  ).not.toBeInTheDocument();
});
