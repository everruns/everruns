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
    await expect(
      page.getByRole("heading", { name: "Canonical component gallery" }),
    ).toBeVisible();
  });

  test("should render the shared chat primitives", async ({ page }) => {
    await expect(page.getByText("Message Info", { exact: true })).toBeVisible();
    await expect(page.getByText("Attachments", { exact: true })).toBeVisible();
    await expect(
      page.getByText("Tool Activity", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Execution Plan", { exact: true }),
    ).toBeVisible();
    await expect(
      page.getByText("Components now follow the canonical chat style."),
    ).toBeVisible();
    await expect(page.getByText("layout.png")).toBeVisible();
    await expect(
      page.getByText("Applying canonical styling").first(),
    ).toBeVisible();
  });

  test("should use self-contained attachment previews in the gallery", async ({
    page,
  }) => {
    const uploadedPreview = page.locator('img[alt="layout.png"]');
    const uploadingPreview = page.locator('img[alt="annotated.jpg"]');

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
    await expect(
      page.getByRole("heading", { name: "Minimal execution states" }),
    ).toBeVisible();
    await expect(
      page.getByText(
        "Can you inspect this project and sketch the rewrite steps?",
      ),
    ).toBeVisible();
    await expect(
      page.getByText("Rewrite it so it supports a rock collection instead."),
    ).toBeVisible();
    await expect(page.getByText("/ship")).toBeVisible();
    await expect(page.getByText("Pick File")).toBeVisible();
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

    await page.getByRole("link", { name: /Chat Components/i }).click();

    await expect(page).toHaveURL("/dev/chat-components");
    await expect(
      page.getByRole("heading", { name: "Canonical component gallery" }),
    ).toBeVisible();
  });
});
