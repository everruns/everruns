import { fireEvent, render, screen } from "@testing-library/react";
import {
  SlackConversationSettings,
  SlackPrivateApprovalSettings,
} from "@/components/apps/slack-conversation-settings";

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

it("can enable and disable private approval cards without implying private agent replies", () => {
  const onChange = jest.fn();
  const { rerender } = render(<SlackPrivateApprovalSettings enabled={false} onChange={onChange} />);
  const toggle = screen.getByRole("switch", { name: "Private approval cards" });
  expect(toggle).not.toBeChecked();
  fireEvent.click(toggle);
  expect(onChange).toHaveBeenLastCalledWith(true);
  rerender(<SlackPrivateApprovalSettings enabled={true} onChange={onChange} />);
  expect(toggle).toBeChecked();
  fireEvent.click(toggle);
  expect(onChange).toHaveBeenLastCalledWith(false);
  expect(screen.getByText(/Agent replies remain visible/)).toBeVisible();
});
