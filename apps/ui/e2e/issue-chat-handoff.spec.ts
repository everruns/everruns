import { expect, test } from "@playwright/test";
import { mockKnowledgeApi } from "./fixtures/knowledge-app";

test("Health continues in Main chat with editable context and preserves an unsent question", async ({ page, context, baseURL }) => {
  await context.addCookies([{ name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" }]);
  await mockKnowledgeApi(page, { layout: "classic" });
  const issue = {
    id: "00000000-0000-7000-8000-000000000001", code: "agent.unattended_spend_spike", status: "open", stale: false,
    agent_id: "agent_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb", agent_name: "Housekeeping", channel_id: null, subject_id: "agent_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    title: "Unattended spend jumped", body: "Housekeeping spent $6.61 on unattended work in the last 24 hours.",
    missing_scopes: [], error_code: null, fixers: [], evidence: null, notification_id: null, snoozed_until: null,
    first_detected_at: "2026-11-25T10:00:00Z", last_checked_at: "2026-11-25T11:00:00Z", href: "/settings/health?issue=00000000-0000-7000-8000-000000000001",
  };
  await page.route("**/api/v1/platform/agents/config", route => route.fulfill({ json: { policies: { "agent.view": true, "agent.manage": true } } }));
  await page.route("**/api/v1/platform/health-issues**", route => route.fulfill({ json: new URL(route.request().url()).pathname.endsWith(issue.id) ? issue : { data: [issue], total: 1, limit: 20, offset: 0 } }));
  let messagesSent = 0;
  page.on("request", request => {
    if (request.method() === "POST" && new URL(request.url()).pathname.endsWith("/messages")) messagesSent++;
  });
  await page.goto("/chats");
  const chat = page.getByRole("region", { name: "Platform Chat", exact: true });
  await chat.getByRole("combobox").fill("Please keep my unsent question.");
  await page.goto(issue.href);
  await page.getByRole("button", { name: "Continue in chat", exact: true }).click();
  await page.waitForURL(/\/chats$/);
  const draft = chat.getByRole("combobox");
  await expect(draft).toHaveValue(/Please keep my unsent question\./);
  await expect(draft).toHaveValue(/Unattended spend jumped/);
  await expect(draft).toHaveValue(/Housekeeping spent \$6\.61/);
  await expect(draft).toHaveValue(new RegExp(issue.id));
  expect(messagesSent).toBe(0);
  expect(page.url()).not.toContain("Housekeeping");
  await page.reload();
  await expect(chat.getByRole("combobox")).toHaveValue(/Unattended spend jumped/);
  expect(messagesSent).toBe(0);
});
