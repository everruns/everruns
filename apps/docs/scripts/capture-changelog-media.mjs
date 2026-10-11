#!/usr/bin/env node
// Captures sharp screenshots and videos for changelog entries.
//
// Decision: every capture runs at device scale factor 2 in the light theme, so
// a screenshot cropped to 700 CSS pixels is a 1400-pixel PNG that stays crisp
// on high-density screens, and video frames come from the browser's own 2x
// screencast instead of a downscaled screen recording. Output goes straight
// into the entry's folder; check-changelog.mjs enforces the same rules.
//
// Usage:
//   node scripts/capture-changelog-media.mjs shot <url> <entry-folder>/<name>@2x.png \
//     [--selector "<css>"] [--clip x,y,width,height] [--viewport 1280x800] \
//     [--wait "<css>"] [--theme light|dark] [--padding 16]
//   node scripts/capture-changelog-media.mjs video <steps.json> <entry-folder>/<name>@2x.mp4
//
// steps.json: { "viewport": "1280x800", "clip": [x,y,w,h]?, "steps": [
//   { "goto": "http://localhost:27100/agents" }, { "wait": "text=New agent" },
//   { "click": "text=New agent" }, { "type": ["#name", "Support bot"] },
//   { "pause": 800 } ] }
// The video also writes <name>-poster@2x.png from its last frame.
// Embed it in the entry as:
//   <video src="./<name>@2x.mp4" poster="./<name>-poster@2x.png" controls muted playsinline loop></video>
//
// Needs Playwright from apps/ui (`pnpm install` there) and ffmpeg for video.
import { execFileSync } from "node:child_process";
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const repoRoot = path.resolve(__dirname, "../../..");
const requireFromUi = createRequire(path.join(repoRoot, "apps/ui/package.json"));
const { chromium } = requireFromUi("playwright");

const SCALE = 2;

function parseArgs(argv) {
  const options = {};
  const positional = [];
  for (let i = 0; i < argv.length; i++) {
    if (argv[i].startsWith("--")) options[argv[i].slice(2)] = argv[++i];
    else positional.push(argv[i]);
  }
  return { positional, options };
}

const size = (value, fallback) => {
  const [width, height] = (value ?? fallback).split("x").map(Number);
  return { width, height };
};

async function openPage(viewport, theme = "light") {
  const browser = await chromium.launch({
    executablePath: process.env.CHROMIUM_PATH || undefined,
  });
  const context = await browser.newContext({
    viewport,
    deviceScaleFactor: SCALE,
    colorScheme: theme,
    reducedMotion: "no-preference",
  });
  const page = await context.newPage();
  return { browser, page };
}

function assertTarget(out, ext) {
  if (!new RegExp(`@2x\\.${ext}$`).test(out)) {
    throw new Error(`${out}: name changelog media name@2x.${ext}`);
  }
}

async function shot(url, out, options) {
  assertTarget(out, "png");
  const { browser, page } = await openPage(size(options.viewport, "1280x800"), options.theme);
  try {
    await page.goto(url, { waitUntil: "networkidle" });
    if (options.wait) await page.waitForSelector(options.wait, { timeout: 30_000 });
    await page.evaluate(() => document.fonts.ready);
    const padding = Number(options.padding ?? 16);
    if (options.selector) {
      const box = await page.locator(options.selector).first().boundingBox();
      if (!box) throw new Error(`${options.selector} is not visible`);
      await page.screenshot({
        path: out,
        clip: {
          x: Math.max(0, box.x - padding),
          y: Math.max(0, box.y - padding),
          width: box.width + padding * 2,
          height: box.height + padding * 2,
        },
      });
    } else if (options.clip) {
      const [x, y, width, height] = options.clip.split(",").map(Number);
      await page.screenshot({ path: out, clip: { x, y, width, height } });
    } else {
      await page.screenshot({ path: out });
    }
  } finally {
    await browser.close();
  }
  console.log(`wrote ${out}`);
}

async function video(stepsFile, out) {
  assertTarget(out, "mp4");
  const script = JSON.parse(readFileSync(stepsFile, "utf8"));
  const viewport = size(script.viewport, "1280x800");
  const { browser, page } = await openPage(viewport, script.theme);
  const frames = [];
  const work = mkdtempSync(path.join(tmpdir(), "changelog-video-"));
  try {
    const first = script.steps.find((s) => s.goto);
    if (first) await page.goto(first.goto, { waitUntil: "networkidle" });
    const cdp = await page.context().newCDPSession(page);
    cdp.on("Page.screencastFrame", async ({ data, metadata, sessionId }) => {
      frames.push({ data, time: metadata.timestamp });
      await cdp.send("Page.screencastFrameAck", { sessionId }).catch(() => {});
    });
    await cdp.send("Page.startScreencast", {
      format: "png",
      maxWidth: viewport.width * SCALE,
      maxHeight: viewport.height * SCALE,
      everyNthFrame: 1,
    });
    await page.waitForTimeout(400);
    for (const step of script.steps) {
      if (step === first) continue;
      if (step.goto) await page.goto(step.goto, { waitUntil: "networkidle" });
      if (step.wait) await page.waitForSelector(step.wait, { timeout: 30_000 });
      if (step.click) await page.click(step.click);
      if (step.hover) await page.hover(step.hover);
      if (step.type) await page.locator(step.type[0]).pressSequentially(step.type[1], { delay: 45 });
      if (step.press) await page.keyboard.press(step.press);
      await page.waitForTimeout(step.pause ?? 500);
    }
    await page.waitForTimeout(800);
    await cdp.send("Page.stopScreencast");
  } finally {
    await browser.close();
  }
  if (frames.length < 2) throw new Error("no frames captured");

  // Screencast frames arrive only when the page repaints, so each frame is
  // held until the next one: an ffconcat list with per-frame durations.
  const lines = ["ffconcat version 1.0"];
  frames.forEach((frame, i) => {
    const file = path.join(work, `f${String(i).padStart(5, "0")}.png`);
    writeFileSync(file, Buffer.from(frame.data, "base64"));
    const next = frames[i + 1]?.time ?? frame.time + 1;
    lines.push(`file '${file}'`, `duration ${Math.max(0.02, next - frame.time).toFixed(3)}`);
  });
  lines.push(`file '${path.join(work, `f${String(frames.length - 1).padStart(5, "0")}.png`)}'`);
  const list = path.join(work, "frames.ffconcat");
  writeFileSync(list, lines.join("\n"));

  let [x, y, w, h] = script.clip ?? [0, 0, viewport.width, viewport.height];
  [x, y, w, h] = [x, y, w, h].map((v) => Math.round((v * SCALE) / 2) * 2);
  execFileSync("ffmpeg", [
    "-y", "-loglevel", "error", "-f", "concat", "-safe", "0", "-i", list,
    "-vf", `crop=${w}:${h}:${x}:${y},fps=30`,
    "-c:v", "libx264", "-preset", "slow", "-crf", "16", "-tune", "stillimage",
    "-pix_fmt", "yuv420p", "-movflags", "+faststart", "-an", out,
  ]);
  const poster = out.replace(/@2x\.mp4$/, "-poster@2x.png");
  // The poster comes from the last lossless frame, not the encoded video.
  const last = path.join(work, `f${String(frames.length - 1).padStart(5, "0")}.png`);
  execFileSync("ffmpeg", [
    "-y", "-loglevel", "error", "-i", last, "-vf", `crop=${w}:${h}:${x}:${y}`, poster,
  ]);
  rmSync(work, { recursive: true, force: true });
  console.log(`wrote ${out} (${frames.length} frames) and ${poster}`);
}

const { positional, options } = parseArgs(process.argv.slice(2));
const [command, a, b] = positional;
if (command === "shot" && a && b) await shot(a, b, options);
else if (command === "video" && a && b) await video(a, b);
else {
  console.error("usage: capture-changelog-media.mjs shot <url> <out@2x.png> [options] | video <steps.json> <out@2x.mp4>");
  process.exit(2);
}
