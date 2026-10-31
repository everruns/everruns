// Capture the production console renderer, using the catalog's browser fixtures.
// Start the console dev server first; pass its origin as the first argument.
import { chromium } from "@playwright/test";
import { mkdir, readFile, writeFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import { createRequire } from "node:module";

const sharp = createRequire(new URL("../../docs/package.json", import.meta.url))("sharp");

const destination = new URL("../../../docs/features/generative-ui-gallery/", import.meta.url);
const samples = JSON.parse(
  await readFile(new URL("../generative-ui/tests/standard-samples.json", import.meta.url)),
);
const { networkExamples } = JSON.parse(
  await readFile(new URL("../generative-ui/tests/gallery-examples.json", import.meta.url)),
);
await mkdir(destination, { recursive: true });
const browser = await chromium.launch({
  executablePath: process.env.PLAYWRIGHT_CHROMIUM_PATH || undefined,
  args: ["--no-sandbox", "--disable-dev-shm-usage"],
});
try {
  const page = await browser.newPage({
    viewport: { width: 960, height: 1000 },
    deviceScaleFactor: 2,
  });
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  page.on("console", (message) => {
    if (message.text().includes("[generativeUI]")) errors.push(message.text());
  });
  await page.route("**/api/**", (route) =>
    route.fulfill({ json: { auth_mode: "none", data: [] } }),
  );
  await page.goto(`${process.argv[2] || "http://localhost:9120"}/dev/generative-ui`);
  await page.locator("[data-generative-ui-root]").last().waitFor({ timeout: 120000 });
  await page.evaluate(() => document.fonts.ready);
  await page.addStyleTag({ content: "nextjs-portal { display: none !important; }" });
  if (await page.getByText("Part of this UI couldn't be shown.", { exact: true }).count())
    throw new Error("Catalog contains a renderer fallback");
  for (const theme of ["light", "dark"]) {
    await page.evaluate((value) => {
      document.documentElement.classList.toggle("dark", value === "dark");
      document.documentElement.style.colorScheme = value;
    }, theme);
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    for (let index = 0; index < samples.length; index++) {
      if (networkExamples.includes(index)) continue;
      const card = page.locator("[data-covers]").nth(index);
      await card.scrollIntoViewIfNeeded();
      await card.evaluate((node) => {
        node.style.padding = "20px";
        node.style.background = "hsl(var(--background))";
        node.querySelector("h2").style.display = "none";
      });
      // Let responsive charts, images and theme observers settle after scrolling.
      await page.waitForTimeout(200);
      const png = await card.screenshot({ animations: "disabled" });
      // sharp is owned by the docs application, where these assets are published.
      await sharp(png)
        .webp({ quality: 92, smartSubsample: true })
        .toFile(fileURLToPath(new URL(`${index}-${theme}.webp`, destination)));
    }
  }
  if (errors.length) throw new Error(errors.join("\n"));
  await writeFile(
    new URL("capture.json", destination),
    JSON.stringify(
      {
        catalog: "copilotkit/standard@1",
        samples: samples.length,
        components: [...new Set(samples.flatMap((sample) => sample.covers))].sort(),
      },
      null,
      2,
    ) + "\n",
  );
  console.log(
    `Captured ${samples.length - networkExamples.length} examples in light and dark themes; external-media examples retain source only.`,
  );
} finally {
  await browser.close();
}
