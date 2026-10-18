import {
  CRON_MIN_INTERVAL_SECONDS,
  describeCronExpression,
  getCronIntervalSeconds,
  isSupportedCronExpression,
} from "@/components/apps/cron-label";

describe("getCronIntervalSeconds", () => {
  it("returns 60 for every-minute 7-field expression", () => {
    expect(getCronIntervalSeconds("0 * * * * * *")).toBe(60);
  });

  it("returns 300 for every-5-minute 7-field expression", () => {
    expect(getCronIntervalSeconds("0 */5 * * * * *")).toBe(300);
  });

  it("returns 3600 for hourly expression", () => {
    expect(getCronIntervalSeconds("0 0 * * * * *")).toBe(3600);
  });

  it("returns 300 for every-5-minute 5-field expression", () => {
    expect(getCronIntervalSeconds("*/5 * * * *")).toBe(300);
  });

  it("returns the minimum gap for a minute-list with uneven spacing (0,4 -> 240s, 56s)", () => {
    // Minutes 0 and 4 of every hour: gaps are 4 min (240s) and 56 min (3360s).
    // The minimum is 240s, which is below CRON_MIN_INTERVAL_SECONDS (300s).
    const secs = getCronIntervalSeconds("0,4 * * * *");
    expect(secs).not.toBeNull();
    expect(secs!).toBeLessThan(CRON_MIN_INTERVAL_SECONDS);
  });

  it("returns null for an invalid expression", () => {
    expect(getCronIntervalSeconds("not a cron")).toBeNull();
  });
});

describe("isSupportedCronExpression", () => {
  it("accepts the default 5-min expression", () => {
    expect(isSupportedCronExpression("0 */5 * * * * *")).toBe(true);
  });

  it("rejects an invalid expression", () => {
    expect(isSupportedCronExpression("bad")).toBe(false);
  });
});

// EVE-1260: the server's `cron` crate numbers 7-field weekdays 1-7 from Sunday;
// 5-field cron is standard (0-6 from Sunday). The preview must say what runs.
describe("describeCronExpression weekdays", () => {
  it("reads 7-field weekdays the cron crate's way", () => {
    expect(describeCronExpression("0 0 9 * * 2-6 *")).toBe("At 09:00 AM, Monday through Friday");
    expect(describeCronExpression("0 0 9 * * 1-5 *")).toBe("At 09:00 AM, Sunday through Thursday");
    expect(describeCronExpression("0 0 18 * * 1 *")).toBe("At 06:00 PM, only on Sunday");
  });

  it("keeps 5-field weekdays standard", () => {
    expect(describeCronExpression("0 9 * * 1-5")).toBe("At 09:00 AM, Monday through Friday");
    expect(describeCronExpression("0 18 * * 0")).toBe("At 06:00 PM, only on Sunday");
  });

  it("rejects a 7-field weekday the server rejects", () => {
    expect(isSupportedCronExpression("0 0 9 * * 0 *")).toBe(false);
  });

  it("previews 7-field named weekdays unchanged", () => {
    expect(describeCronExpression("0 0 9 * * MON-FRI *")).toBe(
      "At 09:00 AM, Monday through Friday",
    );
  });
});
