import { expect, test } from "@playwright/test";

test.use({ viewport: { width: 1280, height: 1000 }, deviceScaleFactor: 2 });

for (const theme of ["light", "dark"]) {
  test(`approval controls stay visible and decisions stay unchanged (${theme})`, async ({
    page,
  }) => {
    test.setTimeout(60_000);
    await page.goto("/dev/tool-activity");
    await page.evaluate(
      (theme) => document.documentElement.classList.toggle("dark", theme === "dark"),
      theme,
    );
    const section = page.getByRole("region", { name: "Approval control characters" });
    await expect(
      section.getByRole("img", { name: "Unicode control character U+202E" }),
    ).toHaveCount(2);
    await expect(
      section.getByRole("img", { name: "Unicode control character U+2066" }),
    ).toBeVisible();
    expect(await section.textContent()).not.toMatch(/[\u202E\u2066]/u);
    await expect(section).toContainText("می‌خواهم 👩‍💻");
    await section.screenshot({ path: `/tmp/approval-controls-${theme}@2x.png` });

    await page.route(
      "**/api/v1/sessions/approval-display-preview/tool-approvals",
      async (route) => {
        expect(route.request().postDataJSON()).toEqual({
          decisions: [{ tool_call_id: "tool_approval_display_preview", decision: "allow" }],
        });
        await route.fulfill({
          json: {
            status: "active",
            resolved: [
              {
                tool_call_id: "tool_approval_display_preview",
                outcome: "allow",
                tool: "delete_file",
              },
            ],
          },
        });
      },
    );
    await section.getByRole("button", { name: "Allow once", exact: true }).click();
    await expect(section.getByText("Allowed once: Delete file")).toBeVisible();

    await page.route("**/api/v1/sessions/approval-display-preview/tool-results", async (route) => {
      expect(route.request().postDataJSON()).toEqual({
        tool_results: [
          { tool_call_id: "mcp_approval_display_preview", result: { approve: false } },
        ],
      });
      await route.fulfill({ json: { status: "running", tool_results_count: 1 } });
    });
    await section.getByRole("button", { name: "Deny", exact: true }).click();
    await expect(section).toContainText("Denied read\\u2066file on Documents");
    expect(await section.textContent()).not.toContain("\u2066");
  });
}
