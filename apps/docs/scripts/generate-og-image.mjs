// Generates the social cards (og:image) for everruns.com/docs:
//   1. Docs home / fallback → public/og-image.png
//   2. Per API operation    → public/og/api/operations/{operationId}.png
//   3. Per docs page        → public/og/{slug}.png
//   4. Changelog index      → public/og/changelog.png, listing the latest releases
//   5. Changelog entries    → public/og/changelog/{folder}.png, with date and products
//
// Cards follow the docs redesign: flat navy, the gold mark, Geist type, square
// corners. Text is rendered by sharp (librsvg) through fontconfig, pointed at
// the Geist TTFs vendored in src/assets/fonts/ (OFL), so the cards look the
// same on every machine instead of falling back to whatever sans-serif the
// build host has. FONTCONFIG_FILE must be set before sharp loads.

import { mkdirSync, existsSync, readdirSync, readFileSync, writeFileSync, mkdtempSync } from "node:fs";
import { fileURLToPath } from "node:url";
import os from "node:os";
import path from "node:path";

import { JSON_SCHEMA, load } from "js-yaml";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");
const repoRoot = path.resolve(root, "../..");
const fontsDir = path.join(root, "src/assets/fonts");

const fcDir = mkdtempSync(path.join(os.tmpdir(), "og-fonts-"));
writeFileSync(
  path.join(fcDir, "fonts.conf"),
  `<?xml version="1.0"?><!DOCTYPE fontconfig SYSTEM "fonts.dtd"><fontconfig><dir>${fontsDir}</dir><cachedir>${fcDir}/cache</cachedir></fontconfig>`,
);
process.env.FONTCONFIG_FILE = path.join(fcDir, "fonts.conf");
const { default: sharp } = await import("sharp");

// ─── Shared look ────────────────────────────────────────────────────────────

const W = 1200;
const H = 630;
const PAD = 72;
const NAVY = "#0A1636";
const GOLD = "#D4A43A";
const WHITE = "#FFFFFF";
const SILVER = "#A8B0BE";
const MUTED = "#7D879A";
const RULE = "rgba(255,255,255,0.12)";
const SANS = "Geist";
const MONO = "Geist Mono";

const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const PRODUCTS = {
  cloud: "Cloud",
  platform: "Platform",
  framework: "Framework",
  sdk: "SDK",
  cli: "CLI",
  yolop: "Yolop",
  mira: "Mira",
  bashkit: "Bashkit",
};

function escXml(s) {
  return String(s).replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function truncate(s, maxLen) {
  return s.length > maxLen ? s.slice(0, maxLen - 1).trimEnd() + "…" : s;
}

// Geist's average advance is about 0.5em for text at weight 400 to 600; this
// is a little generous so wrapped lines never run past the column.
const charsFor = (width, fontSize) => Math.floor(width / (fontSize * 0.52));

function wrapText(text, width, fontSize, maxLines) {
  const maxChars = charsFor(width, fontSize);
  const lines = [];
  let current = "";
  for (const word of text.split(/\s+/).filter(Boolean)) {
    if (current && current.length + 1 + word.length > maxChars) {
      lines.push(current);
      current = word;
    } else {
      current = current ? `${current} ${word}` : word;
    }
  }
  if (current) lines.push(current);
  if (lines.length > maxLines) {
    const kept = lines.slice(0, maxLines);
    kept[maxLines - 1] = truncate(`${kept[maxLines - 1]} ${lines[maxLines]}`, maxChars);
    if (!kept[maxLines - 1].endsWith("…")) kept[maxLines - 1] += "…";
    return kept;
  }
  return lines;
}

function formatDate(iso) {
  const [y, m, d] = String(iso).split("-").map(Number);
  return `${MONTHS[m - 1]} ${d}, ${y}`;
}

function textLines(lines, { x, y, size, lineHeight, weight = 400, fill, family = SANS, spacing = 0 }) {
  return lines
    .map(
      (line, i) =>
        `<text x="${x}" y="${y + i * lineHeight}" font-family="${family}" font-size="${size}" font-weight="${weight}" letter-spacing="${spacing}" fill="${fill}" xml:space="preserve">${escXml(line)}</text>`,
    )
    .join("\n  ");
}

// The mark: three gold rings, as on everruns.com's cards.
function mark(x, y, size) {
  const s = size / 512;
  return `<g transform="translate(${x}, ${y}) scale(${s})" fill="none" stroke-width="22" stroke-linecap="round">
    <circle cx="256" cy="196" r="120" stroke="${GOLD}"/>
    <circle cx="186" cy="291" r="120" stroke="${GOLD}" opacity="0.7"/>
    <circle cx="326" cy="291" r="120" stroke="${GOLD}" opacity="0.7"/>
  </g>`;
}

/** Card frame: background, brand row with an optional eyebrow, footer with the URL. */
function frame(content, { eyebrow = "", url = "everruns.com/docs" } = {}) {
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${W}" height="${H}" viewBox="0 0 ${W} ${H}">
  <rect width="${W}" height="${H}" fill="${NAVY}"/>
  ${mark(PAD - 4, 52, 44)}
  <text x="${PAD + 50}" y="83" font-family="${SANS}" font-size="26" font-weight="600" letter-spacing="-0.4" fill="${WHITE}">Everruns</text>
  <text x="${PAD + 172}" y="83" font-family="${SANS}" font-size="26" font-weight="400" fill="${SILVER}">Docs</text>
  ${eyebrow ? `<text x="${W - PAD}" y="82" text-anchor="end" font-family="${MONO}" font-size="17" font-weight="500" letter-spacing="2" fill="${GOLD}">${escXml(eyebrow.toUpperCase())}</text>` : ""}
  ${content}
  <rect x="${PAD}" y="${H - 86}" width="${W - PAD * 2}" height="1" fill="${RULE}"/>
  <text x="${PAD}" y="${H - 44}" font-family="${MONO}" font-size="19" fill="${MUTED}">${escXml(url)}</text>
</svg>`;
}

/** Title and description, starting at y, within width. Returns { svg, bottom }. */
function titleBlock(title, description, { y = 196, width = W - PAD * 2, maxTitleLines = 3 } = {}) {
  const size = title.length > 48 ? 54 : 64;
  const lineHeight = Math.round(size * 1.1);
  const titleLines = wrapText(title, width, size, maxTitleLines);
  let svg = textLines(titleLines, { x: PAD, y, size, lineHeight, weight: 600, fill: WHITE, spacing: -size * 0.02 });
  let bottom = y + (titleLines.length - 1) * lineHeight;
  if (description) {
    const descY = bottom + 58;
    const descLines = wrapText(description, width, 26, titleLines.length > 2 ? 2 : titleLines.length === 1 ? 4 : 3);
    svg += "\n  " + textLines(descLines, { x: PAD, y: descY, size: 26, lineHeight: 37, fill: SILVER });
    bottom = descY + (descLines.length - 1) * 37;
  }
  return { svg, bottom };
}

async function svgToPng(svg, outPath, composite = []) {
  mkdirSync(path.dirname(outPath), { recursive: true });
  await sharp(Buffer.from(svg)).png().composite(composite).toFile(outPath);
}

function readEntry(filePath) {
  const content = readFileSync(filePath, "utf8");
  const match = content.match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!match) return null;
  let fm;
  try {
    fm = load(match[1], { schema: JSON_SCHEMA }) ?? {};
  } catch {
    return null;
  }
  return { fm, body: match[2] };
}

// ─── 1. Docs home / fallback ────────────────────────────────────────────────

async function generateFallback() {
  const { svg } = titleBlock(
    "Build, run and operate durable agents",
    "Tutorials, how-to guides, reference and explanation for the Everruns Framework, the self-hosted Platform and Everruns Cloud.",
    { y: 250 },
  );
  await svgToPng(frame(svg, { eyebrow: "Documentation" }), path.join(root, "public/og-image.png"));
  console.log("  ✓ fallback og-image.png");
}

// ─── 2. Per-API-operation cards ─────────────────────────────────────────────

const METHOD_COLORS = {
  GET: "#2E7D4F",
  POST: "#2F5FD0",
  PUT: "#B4651F",
  PATCH: "#7650C9",
  DELETE: "#B83A3A",
  HEAD: "#5B6475",
  OPTIONS: "#5B6475",
};

function apiOperationSvg(method, apiPath, description, hasBody) {
  const color = METHOD_COLORS[method] || METHOD_COLORS.GET;
  const desc = description.replace(/^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+\S+\s+-\s*/, "");
  const methodWidth = method.length * 15 + 28;
  const pathSize = apiPath.length > 44 ? 26 : 32;
  const descLines = wrapText(desc, W - PAD * 2, 26, 2);
  const curl = [
    `curl -X ${method} "https://app.everruns.com/api${apiPath.replace(/\{(\w+)\}/g, ":$1")}"`,
    `  -H "Authorization: Bearer $EVERRUNS_API_KEY"`,
    ...(hasBody ? [`  -H "Content-Type: application/json" -d '{…}'`] : []),
  ].map((line, i, all) => (i < all.length - 1 ? `${line} \\` : line));
  const boxY = 338;
  const content = `
  <rect x="${PAD}" y="160" width="${methodWidth}" height="44" fill="${color}"/>
  <text x="${PAD + methodWidth / 2}" y="190" text-anchor="middle" font-family="${MONO}" font-size="20" font-weight="500" letter-spacing="1" fill="${WHITE}">${method}</text>
  <text x="${PAD + methodWidth + 20}" y="193" font-family="${MONO}" font-size="${pathSize}" font-weight="500" fill="${WHITE}">${escXml(truncate(apiPath, 56))}</text>
  ${textLines(descLines, { x: PAD, y: 262, size: 26, lineHeight: 37, fill: SILVER })}
  <rect x="${PAD}" y="${boxY}" width="${W - PAD * 2}" height="${curl.length * 32 + 34}" fill="#0A0A0A"/>
  ${textLines(
    curl.map((l) => truncate(l, 88)),
    { x: PAD + 24, y: boxY + 42, size: 18, lineHeight: 32, fill: "#D4D7DD", family: MONO },
  )}`;
  return frame(content, { eyebrow: "API Reference" });
}

async function generateApiCards() {
  const specPath = path.join(repoRoot, "docs/api/openapi.json");
  if (!existsSync(specPath)) {
    console.log("  ⚠ openapi.json not found, skipping API cards");
    return 0;
  }
  const spec = JSON.parse(readFileSync(specPath, "utf8"));
  let count = 0;
  for (const [apiPath, methods] of Object.entries(spec.paths)) {
    for (const [method, op] of Object.entries(methods)) {
      if (typeof op !== "object" || !op.operationId) continue;
      const svg = apiOperationSvg(method.toUpperCase(), apiPath, op.summary || op.description || op.operationId, !!op.requestBody);
      await svgToPng(svg, path.join(root, `public/og/api/operations/${op.operationId}.png`));
      count++;
    }
  }
  return count;
}

// ─── 3. Per-doc-page cards ──────────────────────────────────────────────────

// Hero area: a white panel on the right for the page's first diagram.
const HERO = { x: 676, y: 150, w: 452, h: 372 };

/**
 * Resolve a hero image from a doc: frontmatter `hero:`, else the first local
 * markdown image in the body. Returns an absolute path or null.
 */
function resolveHeroImage(filePath, fm, body) {
  if (typeof fm.hero === "string") {
    const resolved = path.resolve(path.dirname(filePath), fm.hero);
    if (existsSync(resolved)) return resolved;
  }
  const img = body.match(/!\[.*?\]\(([^)\s]+)/);
  if (img && !img[1].startsWith("http")) {
    const resolved = path.resolve(path.dirname(filePath), img[1]);
    if (existsSync(resolved)) return resolved;
  }
  return null;
}

async function heroComposite(heroPath) {
  const inset = 20;
  const buffer = await sharp(heroPath)
    .resize(HERO.w - inset * 2, HERO.h - inset * 2, { fit: "contain", background: { r: 255, g: 255, b: 255, alpha: 1 } })
    .flatten({ background: "#ffffff" })
    .png()
    .toBuffer();
  return [{ input: buffer, left: HERO.x + inset, top: HERO.y + inset }];
}

function sectionLabel(slug) {
  const parts = slug.split("/");
  return parts.length > 1 ? parts[0].replace(/-/g, " ") : "";
}

async function docPageCard(slug, fm, heroPath) {
  const width = heroPath ? HERO.x - PAD - 40 : W - PAD * 2;
  const { svg } = titleBlock(String(fm.title ?? slug), String(fm.description ?? ""), {
    width,
    y: 210,
  });
  const panel = heroPath ? `<rect x="${HERO.x}" y="${HERO.y}" width="${HERO.w}" height="${HERO.h}" fill="#FFFFFF"/>` : "";
  const out = path.join(root, `public/og/${slug}.png`);
  await svgToPng(frame(`${panel}\n  ${svg}`, { eyebrow: sectionLabel(slug) }), out, heroPath ? await heroComposite(heroPath) : []);
}

// ─── 4 and 5. Changelog ─────────────────────────────────────────────────────

function productsLine(products) {
  return (products ?? []).map((p) => PRODUCTS[p] ?? p).join(" · ");
}

async function changelogEntryCard(folder, fm) {
  const title = String(fm.title ?? folder);
  const { svg, bottom } = titleBlock(title, String(fm.description ?? ""), { y: 228 });
  const meta = `<text x="${PAD}" y="160" font-family="${MONO}" font-size="20" fill="${SILVER}">${escXml(
    `${formatDate(fm.date)}  ·  ${productsLine(fm.products)}`,
  )}</text>`;
  void bottom;
  await svgToPng(
    frame(`${meta}\n  ${svg}`, { eyebrow: fm.kind === "release" ? "Release" : "Changelog", url: "everruns.com/docs/changelog" }),
    path.join(root, `public/og/changelog/${folder}.png`),
  );
}

/** "Everruns 0.47.0: Codex session reliability" → { name: "Everruns 0.47.0", note: "Codex session reliability" } */
function releaseRow(fm) {
  const title = String(fm.title ?? "").replace(/\s+release$/i, "");
  const [name, ...rest] = title.split(":");
  // Without a named highlight, the description's first clause stands in.
  const note = rest.join(":").trim() || String(fm.description ?? "").split(/[,;.]/)[0].trim();
  return { name: name.trim(), note, date: String(fm.date) };
}

async function changelogIndexCard(entries) {
  const releases = entries
    .filter((e) => e.fm.kind === "release")
    .sort((a, b) => String(b.fm.date).localeCompare(String(a.fm.date)) || b.fm.release?.localeCompare?.(a.fm.release, undefined, { numeric: true }) || 0)
    .slice(0, 5)
    .map((e) => releaseRow(e.fm));
  const rowsY = 284;
  const rowH = 58;
  const rows = releases
    .map((r, i) => {
      const y = rowsY + i * rowH;
      return `<rect x="${PAD}" y="${y - 38}" width="${W - PAD * 2}" height="1" fill="${RULE}"/>
  <text x="${PAD}" y="${y}" font-family="${MONO}" font-size="19" fill="${MUTED}">${escXml(formatDate(r.date))}</text>
  <text x="${PAD + 190}" y="${y}" font-family="${SANS}" font-size="25" font-weight="600" fill="${WHITE}">${escXml(r.name)}</text>
  <text x="${PAD + 430}" y="${y}" font-family="${SANS}" font-size="23" fill="${SILVER}">${escXml(truncate(r.note, charsFor(W - PAD * 2 - 430, 23)))}</text>`;
    })
    .join("\n  ");
  const content = `
  <text x="${PAD}" y="196" font-family="${SANS}" font-size="64" font-weight="600" letter-spacing="-1.3" fill="${WHITE}">Changelog</text>
  <text x="${W - PAD}" y="196" text-anchor="end" font-family="${MONO}" font-size="17" font-weight="500" letter-spacing="2" fill="${MUTED}">LATEST RELEASES</text>
  ${rows}`;
  await svgToPng(frame(content, { eyebrow: "", url: "everruns.com/docs/changelog" }), path.join(root, "public/og/changelog.png"));
  return releases.length;
}

// ─── Docs walk ──────────────────────────────────────────────────────────────

async function generateDocCards() {
  const docsDir = path.join(repoRoot, "docs");
  if (!existsSync(docsDir)) {
    console.log("  ⚠ docs/ directory not found, skipping doc cards");
    return { pages: 0, heroes: 0, entries: 0, releases: 0 };
  }
  const files = [];
  (function scan(dir, prefix) {
    for (const entry of readdirSync(dir, { withFileTypes: true })) {
      const full = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        // API pages come from the OpenAPI spec (section 2).
        if (!(prefix === "" && entry.name === "api")) scan(full, prefix ? `${prefix}/${entry.name}` : entry.name);
      } else if (/\.mdx?$/.test(entry.name)) {
        const slug = (prefix ? `${prefix}/${entry.name}` : entry.name).replace(/\.mdx?$/, "").replace(/\/?index$/, "");
        files.push({ full, slug });
      }
    }
  })(docsDir, "");

  const entries = [];
  let pages = 0;
  let heroes = 0;
  for (const { full, slug } of files) {
    const parsed = readEntry(full);
    if (!parsed) continue;
    const { fm, body } = parsed;
    const entryMatch = slug.match(/^changelog\/(\d{4}-\d{2}-\d{2}-[a-z0-9-]+)$/);
    if (entryMatch) {
      entries.push({ folder: entryMatch[1], fm });
      await changelogEntryCard(entryMatch[1], fm);
      continue;
    }
    if (slug === "changelog") continue; // the index card lists releases, below
    const heroPath = resolveHeroImage(full, fm, body);
    if (heroPath) heroes++;
    await docPageCard(slug || "index", fm, heroPath);
    pages++;
  }
  const releases = await changelogIndexCard(entries);
  return { pages, heroes, entries: entries.length, releases };
}

// ─── Main ───────────────────────────────────────────────────────────────────

console.log("Generating OG images…");
await generateFallback();
const apiCount = await generateApiCards();
console.log(`  ✓ ${apiCount} API operation cards`);
const docs = await generateDocCards();
console.log(`  ✓ ${docs.pages} doc page cards (${docs.heroes} with hero images)`);
console.log(`  ✓ ${docs.entries} changelog entry cards, changelog index with ${docs.releases} releases`);
console.log("Done.");
