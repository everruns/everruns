import { expect, test, type BrowserContext, type Page } from "@playwright/test";
import { PLAYBOOKS_ID, mockKnowledgeApi } from "./fixtures/knowledge-app";

// Classic keeps the left sidebar; floating and docked Chat sit next to the page
// without leaving it (knowledge/ui/chat-mode.md). Fictional API fixtures only.
async function open(
  context: BrowserContext,
  page: Page,
  baseURL: string,
  classicChat: "floating" | "docked",
) {
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL).hostname, path: "/" },
  ]);
  await mockKnowledgeApi(page, { layout: "classic", classicChat });
  await page.setViewportSize({ width: 1440, height: 900 });
  await page.goto(`/knowledge/${PLAYBOOKS_ID}?file=%2Fvip-escalation.md`);
  await expect(page.getByText("Budgets cap what a Session may spend.")).toBeVisible();
}

test("floating Chat opens over the page, moves, and keeps the page usable", async ({
  context,
  page,
  baseURL,
}, testInfo) => {
  test.setTimeout(60_000);
  await open(context, page, baseURL!, "floating");
  const shell = page.locator(".chat-mode-shell");
  await expect(shell).toHaveAttribute("data-layout", "classic");
  const chat = page.getByRole("region", { name: "Platform Chat", exact: true });
  await expect(chat).toBeHidden();

  await page.getByRole("button", { name: "Show chat" }).click();
  await expect(shell).toHaveAttribute("data-layout", "classic-floating");
  await expect(chat).toBeVisible();
  await expect(page.getByRole("navigation").first()).toBeVisible();
  const before = (await chat.boundingBox())!;
  expect(before.width).toBeGreaterThan(300);
  // The page underneath stays interactive.
  await page.getByRole("link", { name: "Sources" }).click();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/sources`);
  await expect(chat).toBeVisible();

  const handle = page.getByRole("button", { name: "Move chat" });
  const grip = (await handle.boundingBox())!;
  await page.mouse.move(grip.x + grip.width / 2, grip.y + grip.height / 2);
  await page.mouse.down();
  await page.mouse.move(grip.x - 200, grip.y - 40, { steps: 5 });
  await page.mouse.up();
  const after = (await chat.boundingBox())!;
  expect(after.x).toBeLessThan(before.x - 150);
  await testInfo.attach("floating chat", {
    body: await page.screenshot({ path: testInfo.outputPath("floating.png") }),
    contentType: "image/png",
  });

  await page.getByRole("button", { name: "Hide chat" }).click();
  await expect(shell).toHaveAttribute("data-layout", "classic");
  await page.keyboard.press("Control+j");
  await expect(shell).toHaveAttribute("data-layout", "classic-floating");

  await page.getByRole("button", { name: "Full screen", exact: true }).click();
  await page.waitForURL(/\/chats$/);
  await expect(shell).toHaveAttribute("data-layout", "classic-chat");
  await page.getByRole("button", { name: /Show beside page/ }).click();
  await page.waitForURL(`**/knowledge/${PLAYBOOKS_ID}/sources`);
  await expect(shell).toHaveAttribute("data-layout", "classic-floating");
});

test("docked Chat sits right of the page with a resizable divider", async ({
  context,
  page,
  baseURL,
}, testInfo) => {
  test.setTimeout(60_000);
  await open(context, page, baseURL!, "docked");
  const shell = page.locator(".chat-mode-shell");
  await page.keyboard.press("Control+j");
  await expect(shell).toHaveAttribute("data-layout", "classic-docked");
  const chat = page.getByRole("region", { name: "Platform Chat", exact: true });
  const pane = page.getByRole("region", { name: "Current page" });
  await expect(chat).toBeVisible();
  const chatBox = (await chat.boundingBox())!;
  const pageBox = (await pane.boundingBox())!;
  expect(chatBox.x).toBeGreaterThan(pageBox.x + pageBox.width - 1);
  expect(chatBox.x + chatBox.width).toBeGreaterThan(1430);

  const splitter = page.getByRole("separator", { name: "Resize chat" });
  await expect(splitter).toBeVisible();
  const width = chatBox.width;
  await splitter.focus();
  await page.keyboard.press("ArrowLeft");
  await expect.poll(async () => (await chat.boundingBox())!.width).toBeGreaterThan(width + 16);
  await testInfo.attach("docked chat", {
    body: await page.screenshot({ path: testInfo.outputPath("docked.png") }),
    contentType: "image/png",
  });

  // Switching to floating from the header saves the other option.
  await page.getByRole("button", { name: "Float" }).click();
  await expect(shell).toHaveAttribute("data-layout", "classic-floating");
});
