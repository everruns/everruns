import { expect, test } from "@playwright/test";
import { mockKnowledgeApi, PLAYBOOKS_ID } from "./fixtures/knowledge-app";

test.use({ deviceScaleFactor: 2 });

const SESSION = "session_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const TURN = "turn_aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
const token = "sandbox_" + "a".repeat(220);
const report =
  `Початок звіту. Перевіряю стан пісочниці ${token}. ` +
  "Результат перевірки доступний у сесії. ".repeat(45);
function event(sequence: number, type: string, data: object) {
  return {
    id: `responsive-${sequence}`,
    sequence,
    type,
    data,
    session_id: SESSION,
    ts: `2026-11-25T12:00:0${sequence}Z`,
    context: { turn_id: TURN },
  };
}
const events = [
  event(1, "input.message", {
    message: {
      id: "message-user",
      role: "user",
      content: [{ type: "text", text: `User message start ${token}` }],
      metadata: {},
    },
  }),
  event(2, "turn.started", { turn_id: TURN, input_message_id: "message-user" }),
  event(3, "context.compacted", {
    strategy_used: "tool_rebase",
    model: "fixture",
    messages_before: 40,
    messages_after: 40,
    tokens_before: 10_000,
    tokens_after: 8_000,
    duration_ms: 0,
  }),
  event(4, "output.message.completed", {
    message: {
      id: "message-answer",
      role: "assistant",
      content: [
        {
          type: "text",
          text: `Assistant answer start. [A long link](https://example.test/${token})\n\n${report}\n\n\`\`\`text\n${token.repeat(3)}\n\`\`\``,
        },
      ],
      metadata: {},
    },
  }),
  event(5, "turn.completed", { turn_id: TURN }),
  {
    ...event(6, "input.message", {
      message: {
        id: "message-update",
        role: "user",
        content: [{ type: "text", text: report }],
        metadata: { everruns_origin: { kind: "system", name: "Sandbox status check" } },
      },
    }),
    context: {},
  },
];

for (const scenario of ["small-phone", "phone", "docked", "floating"] as const) {
  test(`chat contains long messages and reports in the ${scenario} layout`, async ({
    page,
    context,
    baseURL,
  }, info) => {
    test.setTimeout(60_000);
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
    ]);
    await mockKnowledgeApi(page, {
      layout: "classic",
      classicChat: scenario === "floating" ? "floating" : "docked",
    });
    await page.route(`**/api/v1/platform/sessions/${SESSION}/events**`, (route) =>
      route.fulfill({
        json: { data: events, has_more: false },
        headers: { "x-total-count": String(events.length) },
      }),
    );
    await page.route(`**/api/v1/platform/sessions/${SESSION}/tasks**`, (route) =>
      route.fulfill({
        json: [
          {
            id: "task_responsive",
            session_id: SESSION,
            kind: "subagent",
            state: "succeeded",
            display_name: "Investigate three running sandboxes and their provider records ".repeat(
              4,
            ),
            created_at: "2026-11-25T12:00:03Z",
            started_at: "2026-11-25T12:00:03Z",
            finished_at: "2026-11-25T12:00:04Z",
            updated_at: "2026-11-25T12:00:04Z",
            links: { child_session_id: "session_bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb" },
          },
        ],
      }),
    );
    await page.route("**/api/v1/platform/tasks**", (route) => route.fulfill({ json: [] }));
    await page.setViewportSize(
      scenario.includes("phone")
        ? { width: scenario === "small-phone" ? 320 : 390, height: 844 }
        : { width: 1440, height: 900 },
    );
    await page.goto(scenario.includes("phone") ? "/chats" : `/knowledge/${PLAYBOOKS_ID}`);
    if (!scenario.includes("phone")) await page.getByRole("button", { name: "Show chat" }).click();
    const chat = page.getByRole("region", { name: "Platform Chat", exact: true });
    if (scenario === "docked") {
      await page.getByRole("separator", { name: "Resize chat", exact: true }).focus();
      for (let i = 0; i < 10; i++) await page.keyboard.press("ArrowRight");
    }
    if (scenario === "floating") {
      const grip = await page.locator(".chat-mode-float-resize").boundingBox();
      await page.mouse.move(grip!.x + 5, grip!.y + 5);
      await page.mouse.down();
      await page.mouse.move(grip!.x + 145, grip!.y + 5, { steps: 5 });
      await page.mouse.up();
    }
    await expect(chat.locator("#message-message-user p")).toBeVisible();
    await expect(chat.locator("[data-slot=run-card]")).toHaveCount(1);
    await expect(chat.getByText(/Tool definitions refreshed/)).toBeVisible();
    const update = chat.getByTestId("automatic-update");
    await update.getByRole("button", { name: /Sandbox status check/ }).click();
    await expect(update.getByText(report, { exact: true })).toBeVisible();
    const violations = await chat.evaluate((root) => {
      const pane = root.getBoundingClientRect();
      return [
        ...root.querySelectorAll<HTMLElement>(
          ".chat-transcript-row, [data-slot=run-card], [data-testid=automatic-update], textarea, form button",
        ),
      ]
        .filter((e) => e.getBoundingClientRect().width > 0)
        .filter((e) => {
          const box = e.getBoundingClientRect();
          return (
            box.left < pane.left - 1 ||
            box.right > pane.right + 1 ||
            (e.matches(".chat-transcript-row, [data-testid=automatic-update]") &&
              e.scrollWidth > e.clientWidth + 1)
          );
        })
        .map((e) => ({
          element: e.tagName,
          text: e.textContent?.slice(0, 50),
          width: e.clientWidth,
          scroll: e.scrollWidth,
        }));
    });
    expect(violations).toEqual([]);
    expect(await page.locator("html").evaluate((e) => e.scrollWidth)).toBeLessThanOrEqual(
      scenario === "small-phone" ? 320 : scenario === "phone" ? 390 : 1440,
    );
    await expect(chat.getByRole("combobox")).toBeVisible();
    await info.attach(`${scenario} chat at 2x`, {
      body: await chat.screenshot({ path: info.outputPath(`${scenario}@2x.png`) }),
      contentType: "image/png",
    });
  });
}
