import { APP_NAME, TITLE_SEPARATOR, formatPageTitle } from "@/lib/page-title";

describe("formatPageTitle", () => {
  it("returns just the app name when no segments are provided", () => {
    expect(formatPageTitle()).toBe(APP_NAME);
  });

  it("appends the app name to a single segment", () => {
    expect(formatPageTitle("Agents")).toBe(`Agents${TITLE_SEPARATOR}${APP_NAME}`);
  });

  it("joins multiple segments most-specific-first", () => {
    expect(formatPageTitle("New Agent", "Agents")).toBe(
      `New Agent${TITLE_SEPARATOR}Agents${TITLE_SEPARATOR}${APP_NAME}`,
    );
  });

  it("drops null and undefined segments (loading states)", () => {
    expect(formatPageTitle(null, "Agent")).toBe(`Agent${TITLE_SEPARATOR}${APP_NAME}`);
    expect(formatPageTitle("Edit", undefined, "Agent")).toBe(
      `Edit${TITLE_SEPARATOR}Agent${TITLE_SEPARATOR}${APP_NAME}`,
    );
  });

  it("drops empty and whitespace-only segments and trims the rest", () => {
    expect(formatPageTitle("", "  ", "Agent")).toBe(`Agent${TITLE_SEPARATOR}${APP_NAME}`);
    expect(formatPageTitle("  My Bot  ", "Agent")).toBe(
      `My Bot${TITLE_SEPARATOR}Agent${TITLE_SEPARATOR}${APP_NAME}`,
    );
  });

  it("uses a middle dot separator with single spaces", () => {
    expect(TITLE_SEPARATOR).toBe(" · ");
  });
});
