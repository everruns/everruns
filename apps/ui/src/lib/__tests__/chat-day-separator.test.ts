import { formatDaySeparator, needsDaySeparator } from "../chat-day-separator";

const words = { today: "Today", yesterday: "Yesterday" };
const at = (iso: string) => new Date(iso).getTime();

describe("needsDaySeparator", () => {
  it("opens the transcript with one", () => {
    expect(needsDaySeparator(undefined, at("2026-10-10T10:00:00"))).toBe(true);
  });

  it("skips turns within the hour on the same day", () => {
    expect(needsDaySeparator(at("2026-10-10T10:00:00"), at("2026-10-10T10:59:00"))).toBe(false);
  });

  it("marks a gap over an hour", () => {
    expect(needsDaySeparator(at("2026-10-10T10:00:00"), at("2026-10-10T11:01:00"))).toBe(true);
  });

  it("marks a turn past midnight even when close", () => {
    expect(needsDaySeparator(at("2026-10-09T23:50:00"), at("2026-10-10T00:05:00"))).toBe(true);
  });
});

describe("formatDaySeparator", () => {
  const now = at("2026-10-10T20:00:00");

  it("names today and yesterday", () => {
    expect(formatDaySeparator(at("2026-10-10T19:40:00"), now, "en-US", words)).toBe(
      "Today 7:40 PM",
    );
    expect(formatDaySeparator(at("2026-10-09T09:02:00"), now, "en-US", words)).toBe(
      "Yesterday 9:02 AM",
    );
  });

  it("names the weekday within a week, then the date", () => {
    expect(formatDaySeparator(at("2026-10-07T19:53:00"), now, "en-US", words)).toBe(
      "Wednesday 7:53 PM",
    );
    expect(formatDaySeparator(at("2026-09-28T08:00:00"), now, "en-US", words)).toBe(
      "Sep 28 8:00 AM",
    );
    expect(formatDaySeparator(at("2025-12-31T08:00:00"), now, "en-US", words)).toBe(
      "Dec 31, 2025 8:00 AM",
    );
  });
});
