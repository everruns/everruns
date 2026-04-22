import { render, screen } from "@testing-library/react";
import { ScheduleSetupGuidance } from "@/components/apps/schedule-setup-guidance";

describe("ScheduleSetupGuidance", () => {
  it("renders cron, timezone, and session mode details", () => {
    render(
      <ScheduleSetupGuidance
        cronExpression="0 * * * * * *"
        timezone="America/Chicago"
        sessionMode="shared_session"
        message="Run checks for {{app.name}}"
        isPublished={true}
      />,
    );

    expect(screen.getByText("0 * * * * * *")).toBeInTheDocument();
    expect(screen.getByText("America/Chicago")).toBeInTheDocument();
    expect(screen.getByText("Shared Session")).toBeInTheDocument();
    expect(screen.getByText("This is app-level automation, not the in-session scheduler.")).toBeInTheDocument();
  });
});
