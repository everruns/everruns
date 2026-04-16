import {
  formatScheduleCadence,
  formatScheduledDateTime,
  getScheduleDisplayTitle,
} from "@/lib/schedule-display";

describe("schedule-display", () => {
  it("formats recurring minute intervals compactly", () => {
    expect(formatScheduleCadence({ cronExpression: "*/10 * * * *" })).toBe("Every 10m");
  });

  it("formats daily schedules with a readable time", () => {
    expect(formatScheduleCadence({ cronExpression: "0 9 * * *" })).toBe("Daily 9:00 AM");
  });

  it("formats weekday schedules with a readable prefix", () => {
    expect(formatScheduleCadence({ cronExpression: "30 14 * * 1-5" })).toBe("Weekdays 2:30 PM");
  });

  it("formats one-shot schedules using the provided timezone", () => {
    expect(
      formatScheduleCadence({
        scheduledAt: "2026-04-16T15:30:00Z",
        timezone: "UTC",
      }),
    ).toBe("At Apr 16, 3:30 PM UTC");
  });

  it("formats absolute schedule timestamps", () => {
    expect(formatScheduledDateTime("2026-04-16T15:30:00Z", "UTC")).toBe("Apr 16, 3:30 PM UTC");
  });

  it("extracts background monitor titles from synthetic schedule descriptions", () => {
    expect(
      getScheduleDisplayTitle(`This scheduled monitor fired. Start the background run now.

Use \`spawn_background\` with:
- tool: \`github\`
- title: \`Watch PR 1319\`
- signal_on_completion: true
- args:
\`\`\`json
{}
\`\`\``),
    ).toBe("Watch PR 1319");
  });
});
