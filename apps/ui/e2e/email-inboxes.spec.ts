import { test, expect } from "@playwright/test";
const org = "org_00000000000000000000000000000001";
const inbox = "019a0000-0000-7000-8000-000000000001";
const message = "019a0000-0000-7000-8000-000000000002";
test.use({ viewport: { width: 1440, height: 1100 }, deviceScaleFactor: 2 });
test("email progressive read, reply review and HTML/text template preview", async ({
  page,
  context,
  baseURL,
}, testInfo) => {
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
  ]);
  const reads: string[] = [];
  let sends = 0;
  await page.route("**/api/v1/**", async (route) => {
    const req = route.request();
    const url = new URL(req.url());
    const path = url.pathname.replace(/^\/api\/v1(?:\/platform)?/, "");
    let json: unknown = { data: [], total: 0 };
    if (
      path === "/agent-examples" ||
      path === "/connection-providers" ||
      path.endsWith("/organization-connections")
    )
      json = [];
    else if (path === "/auth/config")
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    else if (path === "/auth/me")
      json = {
        id: "user-1",
        email: "dev@example.com",
        name: "Dev",
        email_verified: true,
        roles: [],
        organizations: [{ public_id: org, name: "Default", role: "owner" }],
      };
    else if (path.endsWith("/feature-flags")) json = {};
    else if (path === `/orgs/${org}`) json = { id: org, name: "Default" };
    else if (path.endsWith("/switch-org")) json = { success: true, org_id: org };
    else if (path === "/email/inboxes")
      json = [
        {
          id: inbox,
          name: "Support inbox",
          address: "support@agentmail.to",
          enabled: true,
          account_id: "account",
        },
      ];
    else if (path === `/email/inboxes/${inbox}/messages`)
      json = [
        {
          id: message,
          subject: "Help with my order",
          sender: "customer@example.com",
          preview: "Where is my order?",
          state: "held",
          reason: "execution_requires_review",
        },
      ];
    else if (path === `/email/inboxes/${inbox}/messages/${message}`) {
      reads.push(url.searchParams.get("view")!);
      json = {
        content:
          url.searchParams.get("view") === "full"
            ? "Where is my order?\n> Previous quoted message"
            : "Where is my order?",
        warnings: [],
        source: "text",
      };
    } else if (path === `/email/inboxes/${inbox}/outbox`)
      json = [
        {
          id: "reply",
          state: sends ? "sent" : "held",
          body: {
            to: ["customer@example.com"],
            subject: "Re: Help with my order",
            text: "We received your request.",
            html: "<p>We can help.</p>",
          },
        },
      ];
    else if (path === `/email/inboxes/${inbox}/outbox/reply/approve`) {
      expect(req.method()).toBe("POST");
      sends++;
      json = { state: "sent" };
    } else if (path === "/email/templates") json = [];
    else if (path === "/email/templates/preview") {
      expect(req.postDataJSON().variables).toEqual({ name: "Alice" });
      json = { subject: "Hello Alice", html: "<p>Hello Alice</p>", text: "Hello Alice" };
    }
    await route.fulfill({ json });
  });
  await page.goto("/email");
  await page.getByRole("button", { name: /Support inbox/ }).click();
  await expect(page.getByText("Where is my order?", { exact: true })).toBeVisible();
  expect(reads).toHaveLength(0);
  await page.getByRole("button", { name: /Help with my order/ }).click();
  expect(reads).toEqual(["clean"]);
  await expect(page.getByText("Previous quoted message", { exact: false })).toHaveCount(0);
  await page.getByRole("button", { name: "Full message", exact: true }).click();
  await expect(page.getByText(/Previous quoted message/)).toBeVisible();
  expect(reads).toEqual(["clean", "full"]);
  expect(sends).toBe(0);
  await page.getByRole("button", { name: "HTML", exact: true }).click();
  await expect(
    page.frameLocator('iframe[title="Email preview"]').getByText("We can help."),
  ).toBeVisible();
  await page.getByRole("button", { name: "Text", exact: true }).click();
  const approve = page.getByRole("button", { name: "Approve and send this reply" });
  await approve.scrollIntoViewIfNeeded();
  await approve.click();
  await expect(page.getByRole("button", { name: "Approve and send this reply" })).toHaveCount(0);
  expect(sends).toBe(1);
  await page.getByRole("button", { name: "Templates", exact: true }).click();
  await page.getByLabel("Template name").fill("Welcome");
  await page.getByLabel("Subject", { exact: true }).fill("Hello {{name}}");
  await page.getByLabel("HTML", { exact: true }).fill("<p>Hello {{name}}</p>");
  await page.getByLabel("Sample variables").fill('{"name":"Alice"}');
  await page.getByRole("button", { name: "Preview", exact: true }).click();
  await expect(page.locator("pre").filter({ hasText: "Hello Alice" })).toBeVisible();
  await expect(
    page.frameLocator('iframe[title="Email preview"]').getByText("Hello Alice"),
  ).toBeVisible();
  await expect(page.locator('iframe[title="Email preview"]')).toHaveAttribute("sandbox", "");
  await page.locator('[data-slot="page-masthead"]').locator("..").screenshot({
    path: testInfo.outputPath("email-template@2x.png"),
    animations: "disabled",
  });
  await page.getByRole("button", { name: "Inboxes", exact: true }).click();
  await page.locator('[data-slot="page-masthead"]').locator("..").screenshot({
    path: testInfo.outputPath("email-inbox@2x.png"),
    animations: "disabled",
  });
});
