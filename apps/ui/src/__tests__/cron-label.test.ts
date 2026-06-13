import {
  CRON_MIN_INTERVAL_SECONDS,
  getCronIntervalSeconds,
  isSupportedCronExpression,
} from "@/components/apps/cron-label";

describe("CRON_MIN_INTERVAL_SECONDS", () => {
  it("equals 300 (matches server-side default)", () => {
    expect(CRON_MIN_INTERVAL_SECONDS).toBe(300);
  });
});

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

  it("default expression 0 */5 * * * * * meets the minimum interval", () => {
    const secs = getCronIntervalSeconds("0 */5 * * * * *");
    expect(secs).not.toBeNull();
    expect(secs!).toBeGreaterThanOrEqual(CRON_MIN_INTERVAL_SECONDS);
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
