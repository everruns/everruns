import { fireEvent, render, screen } from "@testing-library/react";
import { slackInstallNextUrl } from "@/components/agents/channels/slack-install";
import {
  SlackConnectionChoice,
  SlackSecretField,
  defaultSlackConnection,
} from "@/components/agents/channels/slack-socket-mode";

describe("Slack Socket Mode settings", () => {
  it("defaults to Socket Mode when the page is not on public HTTPS", () => {
    // jsdom serves the page from http://localhost.
    expect(defaultSlackConnection()).toBe("socket_mode");
  });

  it("switches between Socket Mode and Request URL", () => {
    const onChange = jest.fn();
    render(<SlackConnectionChoice idPrefix="s" value="socket_mode" onChange={onChange} />);
    expect(screen.getByRole("radio", { name: /Socket Mode/ })).toBeChecked();
    fireEvent.click(screen.getByRole("radio", { name: /Request URL/ }));
    expect(onChange).toHaveBeenCalledWith("request_url");
  });

  it("asks for the secret the chosen connection needs", () => {
    const props = {
      idPrefix: "s",
      mode: "new" as const,
      appId: "A123",
      appToken: "",
      signingSecret: "",
      onAppTokenChange: jest.fn(),
      onSigningSecretChange: jest.fn(),
    };
    const { rerender } = render(<SlackSecretField {...props} connection="socket_mode" />);
    expect(screen.getByLabelText("App-level token")).toHaveAttribute("placeholder", "xapp-...");
    expect(screen.getByRole("link", { name: /Open the app's settings/ })).toHaveAttribute(
      "href",
      "https://api.slack.com/apps/A123/general",
    );
    expect(screen.queryByLabelText("Signing secret")).not.toBeInTheDocument();
    rerender(<SlackSecretField {...props} connection="request_url" />);
    expect(screen.getByLabelText("Signing secret")).toBeInTheDocument();
    expect(screen.queryByLabelText("App-level token")).not.toBeInTheDocument();
  });

  it("returns to the channel when Slack cannot redirect back", () => {
    expect(
      slackInstallNextUrl({ authorize_url: "https://slack.com/oauth/v2/authorize?x" }, "/c"),
    ).toBe("https://slack.com/oauth/v2/authorize?x");
    expect(
      slackInstallNextUrl(
        { install_page_url: "https://api.slack.com/apps/A1/install-on-team" },
        "/c",
      ),
    ).toBe("/c?slack_install=finish");
  });
});
