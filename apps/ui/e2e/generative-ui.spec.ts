import { expect, test } from "@playwright/test";
import samples from "../generative-ui/tests/standard-samples.json";

test("console uses all 69 catalog components with the shared renderer", async ({ page }) => {
  const failures: string[] = [];
  page.on("console", (message) => {
    if (message.text().includes("[generativeUI]")) failures.push(message.text());
  });
  page.on("pageerror", (error) => failures.push(error.message));
  await page.route("**/api/**", (route) =>
    route.fulfill({ json: { auth_mode: "none", data: [] } }),
  );
  await page.goto("/dev/generative-ui");
  await expect(page.getByTestId("generative-ui-catalog")).toBeVisible();
  await expect(page.locator("[data-generative-ui-root]")).toHaveCount(samples.length, {
    timeout: 60000,
  });
  expect(new Set(samples.flatMap((sample) => sample.covers)).size).toBe(69);
  await expect(page.getByText("Part of this UI couldn't be shown.", { exact: true })).toHaveCount(
    0,
  );
  expect(failures).toEqual([]);
  await page.screenshot({ path: "/tmp/everruns-console-catalog.png", fullPage: true });
});
