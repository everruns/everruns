// Run against `pnpm preview` after a production docs build.
import assert from "node:assert/strict";
import { chromium } from "@playwright/test";
import { readFile } from "node:fs/promises";

const origin = process.env.GALLERY_ORIGIN ?? "http://localhost:4322";
const samples = JSON.parse(
  await readFile(new URL("../../ui/generative-ui/tests/standard-samples.json", import.meta.url)),
);
const browser = await chromium.launch({
  args: ["--no-sandbox", "--disable-dev-shm-usage", "--use-angle=swiftshader"],
});
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 900 } });
  const errors = [];
  const writes = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("request", (request) => {
    if (request.method() !== "GET") writes.push(request.url());
  });
  await page.goto(`${origin}/docs/features/generative-ui-gallery/`);
  const search = page.getByPlaceholder("Try Chart, Form, Modal…");
  await search.fill("Form with TextField");
  const card = page.locator("article.gallery-card:visible");
  assert.equal(await card.count(), 1);
  const frame = card.locator("iframe").contentFrame();
  await frame.getByPlaceholder("Your name").fill("Alex");
  await frame.getByPlaceholder("you@company.com").fill("alex@example.com");
  await frame.getByRole("button", { name: "Request quote", exact: true }).click();
  await frame.getByLabel("Example action").filter({ hasText: '"name":"Alex"' }).waitFor();
  await frame.getByRole("button", { name: "Reset", exact: true }).click();
  assert.equal(await frame.getByPlaceholder("Your name").inputValue(), "");
  assert.equal(await frame.getByLabel("Example action").count(), 0);
  await page.evaluate(() => {
    document.documentElement.dataset.theme = "dark";
  });
  await frame.locator("html.dark").waitFor({ state: "attached" });
  await page.screenshot({ path: "/tmp/generative-ui-gallery-live.png", fullPage: false });
  await page.setViewportSize({ width: 390, height: 844 });
  assert.ok(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth));
  await search.fill("Modal");
  const modalFrame = card.locator("iframe").contentFrame();
  await modalFrame.getByRole("button", { name: "Open modal" }).click();
  const embeddedDialog = modalFrame.getByRole("dialog");
  await embeddedDialog.waitFor();
  const embeddedBox = await embeddedDialog.boundingBox();
  const viewportBox = await card.locator("iframe").boundingBox();
  assert.ok(
    embeddedBox &&
      viewportBox &&
      embeddedBox.y >= viewportBox.y &&
      embeddedBox.y + embeddedBox.height <= viewportBox.y + viewportBox.height,
    "embedded modal fits its preview",
  );
  await modalFrame.locator("body").press("Escape");
  await search.fill("Map with markers");
  assert.equal(await card.locator("iframe").count(), 0);
  await card.getByRole("button", { name: /Load interactive example/ }).click();
  assert.equal(await card.locator("iframe").count(), 1);
  await page.close();

  // Exercise every fixture without loading all renderer instances simultaneously.
  const preview = await browser.newPage();
  preview.on("pageerror", (error) => errors.push(error.message));
  for (let index = 0; index < samples.length; index++) {
    await preview.goto(`${origin}/docs/previews/generative-ui/?example=${index}`);
    await preview.locator("[data-generative-ui-root]").waitFor();
    assert.equal(
      await preview.getByText("Part of this UI couldn't be shown.", { exact: true }).count(),
      0,
      samples[index].title,
    );
    if (index === 6) {
      const tabs = preview.getByRole("tab");
      await tabs.last().click();
      assert.equal(await tabs.last().getAttribute("aria-selected"), "true");
    }
    if (index === 38) {
      await preview.getByRole("button", { name: /open/i }).click();
      await preview.getByRole("dialog").waitFor();
      const box = await preview.getByRole("dialog").boundingBox();
      assert.ok(
        box && box.y >= 0 && box.y + box.height <= (await preview.evaluate(() => innerHeight)),
        "modal fits its viewport",
      );
      await preview.keyboard.press("Escape");
      assert.equal(await preview.getByRole("dialog").count(), 0);
    }
  }
  assert.deepEqual(writes, [], "gallery must not create conversations or submit data");
  assert.deepEqual(errors, []);
  console.log(
    `Gallery smoke passed: ${samples.length} examples, ${new Set(samples.flatMap((sample) => sample.covers)).size} components; form, reset, tabs, modal, dark theme, mobile and media activation.`,
  );
} finally {
  await browser.close();
}
