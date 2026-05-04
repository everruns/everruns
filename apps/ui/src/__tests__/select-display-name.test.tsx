import { render, screen } from "@testing-library/react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

// Real-DOM integration test (no base-ui mock) covering the Add Channel
// dropdown bug: <Select> must show the human-readable label of the selected
// item, never the raw internal value. base-ui's <Select.Value> resolves the
// label from the items map our wrapper auto-collects from <SelectItem>
// children. See specs/code-organization.md "Dropdowns Must Show Display
// Names".
describe("Select trigger displays human-readable labels by default", () => {
  it("shows 'Webhook' (label) when value is 'webhook' (internal)", () => {
    render(
      <Select value="webhook" onValueChange={() => {}}>
        <SelectTrigger data-testid="trigger">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="slack">Slack</SelectItem>
          <SelectItem value="ag_ui">AG-UI</SelectItem>
          <SelectItem value="schedule">Schedule</SelectItem>
          <SelectItem value="webhook">Webhook</SelectItem>
        </SelectContent>
      </Select>,
    );

    const trigger = screen.getByTestId("trigger");
    expect(trigger).toHaveTextContent("Webhook");
    expect(trigger).not.toHaveTextContent("webhook");
  });

  it("shows 'Shared Session' (label) when value is 'shared_session' (internal)", () => {
    render(
      <Select value="shared_session" onValueChange={() => {}}>
        <SelectTrigger data-testid="trigger">
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="shared_session">Shared Session</SelectItem>
          <SelectItem value="session_per_invocation">Session Per Invocation</SelectItem>
        </SelectContent>
      </Select>,
    );

    const trigger = screen.getByTestId("trigger");
    expect(trigger).toHaveTextContent("Shared Session");
    expect(trigger).not.toHaveTextContent("shared_session");
  });

  it("shows the placeholder when no value is selected", () => {
    render(
      <Select value="" onValueChange={() => {}}>
        <SelectTrigger data-testid="trigger">
          <SelectValue placeholder="Choose later" />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="slack">Slack</SelectItem>
        </SelectContent>
      </Select>,
    );

    expect(screen.getByTestId("trigger")).toHaveTextContent("Choose later");
  });

  it("explicit children on SelectValue still win over the auto-resolved label", () => {
    render(
      <Select value="agent_123" onValueChange={() => {}}>
        <SelectTrigger data-testid="trigger">
          <SelectValue>Custom display</SelectValue>
        </SelectTrigger>
        <SelectContent>
          <SelectItem value="agent_123">Default Label</SelectItem>
        </SelectContent>
      </Select>,
    );

    expect(screen.getByTestId("trigger")).toHaveTextContent("Custom display");
  });
});
