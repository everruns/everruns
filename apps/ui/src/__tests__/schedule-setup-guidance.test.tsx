import { render, screen } from "@testing-library/react";
import { ScheduleSetupGuidance } from "@/components/agents/integrations/schedule-setup-guidance";

describe("ScheduleSetupGuidance", () => {
  it("renders cron, timezone, and session mode details", () => {
    render(
      <ScheduleSetupGuidance
        cronExpression="0 * * * * * *"
        timezone="America/Chicago"
        sessionMode="shared_session"
        message="Run checks for {{app.name}}"
        isEnabled={true}
      />,
    );

    expect(screen.getByText("0 * * * * * *")).toBeInTheDocument();
    expect(screen.getByText("America/Chicago")).toBeInTheDocument();
    expect(screen.getByText("Shared Session")).toBeInTheDocument();
    expect(
      screen.getByText("This is agent-level automation, not the in-session scheduler."),
    ).toBeInTheDocument();
  });
});
