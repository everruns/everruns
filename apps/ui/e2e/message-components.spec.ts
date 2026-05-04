import { test, expect } from "@playwright/test";

/**
 * E2E smoke tests for chat-related dev pages.
 *
 * These pages exercise the shared chat surface primitives and should stay in
 * sync with the runtime chat experience.
 */

test.describe("Chat Components Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/dev/chat-components");
  });

  test("should render the page title", async ({ page }) => {
    await expect(page.getByRole("heading", { name: "Chat UI" })).toBeVisible();
  });

  test("should render the shared chat primitives", async ({ page }) => {
    await expect(page.getByText("Tool-heavy runtime scene", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Transcript-focused runtime scene", { exact: true }),
    ).toBeVisible();
    await expect(page.getByText("Empty state and composer", { exact: true })).toBeVisible();
    await expect(page.getByText("/ship tighten the tool transcript UI")).toBeVisible();
    await expect(page.getByText("layout.png")).toBeVisible();
  });

  test("should use self-contained attachment previews in the gallery", async ({
    page,
  }) => {
    const uploadedPreview = page.locator('img[alt="layout.png"]');
    const uploadingPreview = page.locator('img[alt="sandbox-daytona.jpg"]');

    await expect(uploadedPreview).toBeVisible();
    await expect(uploadedPreview).toHaveAttribute("src", /data:image\/svg\+xml/);
    await expect(uploadingPreview).toBeVisible();
    await expect(uploadingPreview).toHaveAttribute(
      "src",
      /data:image\/svg\+xml/,
    );
  });
});

test.describe("Tool Activity Page", () => {
  test.beforeEach(async ({ page }) => {
    await page.goto("/dev/tool-activity");
  });

  test("should render the transcript preview", async ({ page }) => {
    await expect(page.getByRole("heading", { name: "Tool Outputs", exact: true })).toBeVisible();
    await expect(page.getByText("Standalone tool outputs", { exact: true })).toBeVisible();
    await expect(page.getByText("Bash output", { exact: true })).toBeVisible();
    await expect(page.getByText("Todo plan output", { exact: true })).toBeVisible();
    await expect(page.getByText("Grouped activity", { exact: true })).toBeVisible();
    await expect(page.getByText("Narrated tool timeline", { exact: true })).toBeVisible();
    await expect(page.getByText("Created Daytona sandbox")).toBeVisible();
    await expect(page.getByText("hello from dev.everruns.com").first()).toBeVisible();
    await expect(page.getByText("Combined transcript grouping", { exact: true })).toBeVisible();
  });
});

test.describe("Dev Index Page", () => {
  test("should render developer tools index", async ({ page }) => {
    await page.goto("/dev");

    await expect(
      page.getByRole("heading", { name: "UI reference pages" }),
    ).toBeVisible();
    await expect(
      page.getByText(
        "Development-only previews for the application’s canonical component styles and behaviors.",
      ),
    ).toBeVisible();
  });

  test("should navigate to chat components page", async ({ page }) => {
    await page.goto("/dev");

    await page.getByRole("link", { name: /Chat UI/i }).click();

    await expect(page).toHaveURL("/dev/chat-components");
    await expect(page.getByRole("heading", { name: "Chat UI" })).toBeVisible();
  });
});
