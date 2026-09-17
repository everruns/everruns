#!/usr/bin/env node

import { readFile, mkdir } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const CSS_VIEWPORT = { width: 1440, height: 900 };
const DEVICE_SCALE_FACTOR = 2;
const OUTPUT_SIZE = {
  width: CSS_VIEWPORT.width * DEVICE_SCALE_FACTOR,
  height: CSS_VIEWPORT.height * DEVICE_SCALE_FACTOR,
};

const scriptDirectory = dirname(fileURLToPath(import.meta.url));
const repositoryRoot = resolve(scriptDirectory, "../../../..");
const requireFromUi = createRequire(resolve(repositoryRoot, "apps/ui/package.json"));

let chromium;
try {
  ({ chromium } = requireFromUi("playwright"));
} catch {
  throw new Error(
    "Playwright is unavailable. Run `pnpm install --frozen-lockfile` in apps/ui first.",
  );
}

const baseUrl = (process.argv[2] ?? "http://localhost:27100").replace(/\/$/, "");
const outputDirectory = resolve(process.argv[3] ?? "assets/screenshots");

async function fetchJson(path) {
  const response = await fetch(`${baseUrl}${path}`);
  if (!response.ok) {
    throw new Error(`${path} returned HTTP ${response.status}`);
  }
  return response.json();
}

async function assertHealthy() {
  const response = await fetch(`${baseUrl}/health`);
  if (!response.ok) {
    throw new Error(`Everruns is not healthy at ${baseUrl} (HTTP ${response.status}).`);
  }
}

async function assertPngSize(path) {
  const header = await readFile(path);
  const width = header.readUInt32BE(16);
  const height = header.readUInt32BE(20);
  if (width !== OUTPUT_SIZE.width || height !== OUTPUT_SIZE.height) {
    throw new Error(
      `${path} is ${width}x${height}; expected ${OUTPUT_SIZE.width}x${OUTPUT_SIZE.height}.`,
    );
  }
}

async function main() {
  await assertHealthy();
  const sessions = await fetchJson("/api/v1/sessions");
  const platformChat = sessions.data?.find(
    (session) =>
      session.title === "Platform Chat" && session.source === "chat" && session.is_pinned === true,
  );
  if (!platformChat) {
    throw new Error("The pinned Platform Chat session was not found.");
  }

  const scenes = [
    ["platform-chat", `/chats/${platformChat.id}`],
    ["sessions", "/sessions"],
    ["agents", "/agents"],
    ["harnesses", "/harnesses"],
    ["durable-execution", "/durable"],
  ];

  await mkdir(outputDirectory, { recursive: true });
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext({
    viewport: CSS_VIEWPORT,
    deviceScaleFactor: DEVICE_SCALE_FACTOR,
  });
  const page = await context.newPage();

  try {
    for (const theme of ["light", "dark"]) {
      await page.emulateMedia({ colorScheme: theme });

      for (const [name, route] of scenes) {
        const output = resolve(outputDirectory, `${name}-${theme}.png`);
        console.log(`Capturing ${name} (${theme}) at ${OUTPUT_SIZE.width}x${OUTPUT_SIZE.height}`);
        await page.goto(`${baseUrl}${route}`, { waitUntil: "networkidle" });
        await page.waitForTimeout(300);

        await page.evaluate(() => {
          window.scrollTo(0, 0);
          document.querySelectorAll("*").forEach((element) => {
            if (element.scrollHeight > element.clientHeight) element.scrollTop = 0;
          });

          // The development portal is browser tooling, not product UI. Presentation screenshots
          // must remain separate from debugging and regression-test evidence.
          document.querySelectorAll("nextjs-portal").forEach((element) => element.remove());
        });

        await page.screenshot({ path: output, animations: "disabled", caret: "hide" });
        await assertPngSize(output);
      }
    }
  } finally {
    await browser.close();
  }

  console.log(
    `Captured ${scenes.length} scenes in light and dark at ${OUTPUT_SIZE.width}x${OUTPUT_SIZE.height} under ${outputDirectory}.`,
  );
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
