// Generates OG social card images:
//   1. Generic fallback  → public/og-image.png
//   2. Per-API-operation  → public/og/api/operations/{operationId}.png
//   3. Per-doc-page       → public/og/{slug}.png
//
// Uses sharp (already a project dependency for Astro image optimization).

import sharp from "sharp";
import { readFileSync, mkdirSync, existsSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";
import { globSync } from "node:fs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const root = path.resolve(__dirname, "..");
const repoRoot = path.resolve(root, "../..");

// ─── Shared SVG fragments ───────────────────────────────────────────────────

const BG = `
  <defs>
    <linearGradient id="bg" x1="0" y1="0" x2="1200" y2="630" gradientUnits="userSpaceOnUse">
      <stop offset="0%" stop-color="#0A1636"/>
      <stop offset="100%" stop-color="#060D1F"/>
    </linearGradient>
    <pattern id="dots" x="0" y="0" width="24" height="24" patternUnits="userSpaceOnUse">
      <circle cx="12" cy="12" r="1" fill="#D4A43A" opacity="0.10"/>
    </pattern>
  </defs>
  <rect width="1200" height="630" fill="url(#bg)"/>
  <rect width="1200" height="630" fill="url(#dots)"/>
  <rect x="0" y="620" width="1200" height="10" fill="#D4A43A" opacity="0.6"/>
`;

const LOGO_SMALL = `
  <g transform="translate(60, 30) scale(0.22)" fill="none" stroke-width="18" stroke-linecap="round" stroke-linejoin="round">
    <defs>
      <linearGradient id="gTop" gradientUnits="userSpaceOnUse" x1="256" y1="94" x2="256" y2="284">
        <stop offset="0.00" stop-color="#0A1636"/><stop offset="0.70" stop-color="#0A1636"/><stop offset="1.00" stop-color="#D4A43A"/>
      </linearGradient>
      <linearGradient id="gLeft" gradientUnits="userSpaceOnUse" x1="70" y1="374" x2="256" y2="284">
        <stop offset="0.00" stop-color="#081C3F"/><stop offset="0.70" stop-color="#081C3F"/><stop offset="1.00" stop-color="#D4A43A"/>
      </linearGradient>
      <linearGradient id="gRight" gradientUnits="userSpaceOnUse" x1="442" y1="374" x2="256" y2="284">
        <stop offset="0.00" stop-color="#0B1233"/><stop offset="0.70" stop-color="#0B1233"/><stop offset="1.00" stop-color="#D4A43A"/>
      </linearGradient>
    </defs>
    <circle cx="256" cy="214" r="120" stroke="url(#gTop)"/>
    <circle cx="186" cy="309" r="120" stroke="url(#gLeft)"/>
    <circle cx="326" cy="309" r="120" stroke="url(#gRight)"/>
  </g>
`;

function escXml(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

/** Truncate text to maxLen characters, adding ellipsis if needed. */
function truncate(s, maxLen) {
  return s.length > maxLen ? s.slice(0, maxLen - 1) + "…" : s;
}

/** Wrap text into lines of approximately maxChars, respecting word boundaries. */
function wrapText(text, maxChars) {
  const words = text.split(/\s+/);
  const lines = [];
  let current = "";
  for (const word of words) {
    if (current && (current.length + 1 + word.length) > maxChars) {
      lines.push(current);
      current = word;
    } else {
      current = current ? current + " " + word : word;
    }
  }
  if (current) lines.push(current);
  return lines;
}

async function svgToPng(svg, outPath) {
  mkdirSync(path.dirname(outPath), { recursive: true });
  await sharp(Buffer.from(svg)).resize(1200, 630).png().toFile(outPath);
}

// ─── 1. Generic fallback card ───────────────────────────────────────────────

async function generateFallback() {
  const svgPath = path.join(root, "src/assets/og-image.svg");
  const svg = readFileSync(svgPath);
  const outPath = path.join(root, "public/og-image.png");
  await sharp(svg).resize(1200, 630).png().toFile(outPath);
  console.log("  ✓ fallback og-image.png");
}

// ─── 2. Per-API-operation cards ─────────────────────────────────────────────

const METHOD_COLORS = {
  GET: { bg: "#22863a", text: "#ffffff" },
  POST: { bg: "#0366d6", text: "#ffffff" },
  PUT: { bg: "#e36209", text: "#ffffff" },
  PATCH: { bg: "#8957e5", text: "#ffffff" },
  DELETE: { bg: "#cb2431", text: "#ffffff" },
  HEAD: { bg: "#6a737d", text: "#ffffff" },
  OPTIONS: { bg: "#6a737d", text: "#ffffff" },
};

function buildCurlExample(method, apiPath, hasBody) {
  const base = "https://app.everruns.com/api";
  // Replace path params {foo} with :foo for readability
  const cleanPath = apiPath.replace(/\{(\w+)\}/g, ":$1");
  let curl = `curl -X ${method} "${base}${cleanPath}"`;
  curl += ` \\\n  -H "Authorization: Bearer $TOKEN"`;
  if (hasBody) {
    curl += ` \\\n  -H "Content-Type: application/json"`;
    curl += ` \\\n  -d '{...}'`;
  }
  return curl;
}

function apiOperationSvg(method, apiPath, description, operationId, hasBody) {
  const mc = METHOD_COLORS[method] || METHOD_COLORS.GET;
  const cleanDesc = truncate(description.replace(/^(GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+\S+\s+-\s*/, ""), 120);
  const descLines = wrapText(cleanDesc, 55);
  const curl = buildCurlExample(method, apiPath, hasBody);
  const curlLines = curl.split("\n").map((l) => escXml(l));

  // Method badge width based on text length
  const methodWidth = method.length * 18 + 24;

  return `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">
  ${BG}
  ${LOGO_SMALL}
  <text x="140" y="62" font-family="system-ui, -apple-system, sans-serif" font-size="22" font-weight="500" fill="#A0A0A0">Everruns</text>
  <text x="260" y="62" font-family="system-ui, -apple-system, sans-serif" font-size="20" font-weight="400" fill="#D4A43A">API Reference</text>

  <!-- Method badge -->
  <rect x="60" y="110" width="${methodWidth}" height="44" rx="6" fill="${mc.bg}"/>
  <text x="${60 + methodWidth / 2}" y="139" font-family="monospace" font-size="22" font-weight="700" fill="${mc.text}" text-anchor="middle">${method}</text>

  <!-- Path -->
  <text x="${60 + methodWidth + 16}" y="140" font-family="monospace" font-size="28" font-weight="500" fill="#FFFFFF">${escXml(apiPath)}</text>

  <!-- Description -->
  ${descLines.map((line, i) => `<text x="60" y="${200 + i * 30}" font-family="system-ui, -apple-system, sans-serif" font-size="22" fill="#A0A0A0">${escXml(line)}</text>`).join("\n  ")}

  <!-- Curl example box -->
  <rect x="60" y="${310}" width="1080" height="${curlLines.length * 28 + 40}" rx="8" fill="#0D1117" stroke="#30363d" stroke-width="1"/>
  <text x="80" y="${338}" font-family="monospace" font-size="14" fill="#6a737d">Example Request</text>
  ${curlLines.map((line, i) => `<text x="80" y="${366 + i * 26}" font-family="monospace" font-size="18" fill="#c9d1d9">${line}</text>`).join("\n  ")}

  <!-- URL -->
  <text x="60" y="590" font-family="system-ui, -apple-system, sans-serif" font-size="18" fill="#D4A43A">docs.everruns.com</text>
</svg>`;
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
      const m = method.toUpperCase();
      const desc = op.summary || op.description || op.operationId;
      const hasBody = !!op.requestBody;
      const svg = apiOperationSvg(m, apiPath, desc, op.operationId, hasBody);
      const outPath = path.join(root, `public/og/api/operations/${op.operationId}.png`);
      await svgToPng(svg, outPath);
      count++;
    }
  }
  return count;
}

// ─── 3. Per-doc-page cards ──────────────────────────────────────────────────

function docPageSvg(title, description, breadcrumb) {
  const titleLines = wrapText(truncate(title, 60), 30);
  const descLines = description ? wrapText(truncate(description, 160), 50) : [];

  return `<svg xmlns="http://www.w3.org/2000/svg" width="1200" height="630" viewBox="0 0 1200 630">
  ${BG}
  ${LOGO_SMALL}
  <text x="140" y="62" font-family="system-ui, -apple-system, sans-serif" font-size="22" font-weight="500" fill="#A0A0A0">Everruns</text>
  ${breadcrumb ? `<text x="260" y="62" font-family="system-ui, -apple-system, sans-serif" font-size="20" font-weight="400" fill="#D4A43A">${escXml(breadcrumb)}</text>` : ""}

  <!-- Title -->
  ${titleLines.map((line, i) => `<text x="60" y="${180 + i * 64}" font-family="system-ui, -apple-system, sans-serif" font-size="56" font-weight="600" fill="#FFFFFF">${escXml(line)}</text>`).join("\n  ")}

  <!-- Description -->
  ${descLines.map((line, i) => `<text x="60" y="${180 + titleLines.length * 64 + 30 + i * 30}" font-family="system-ui, -apple-system, sans-serif" font-size="22" fill="#A0A0A0">${escXml(line)}</text>`).join("\n  ")}

  <!-- URL -->
  <text x="60" y="590" font-family="system-ui, -apple-system, sans-serif" font-size="18" fill="#D4A43A">docs.everruns.com</text>
</svg>`;
}

async function generateDocCards() {
  // Scan markdown docs for frontmatter
  const docsDir = path.join(repoRoot, "docs");
  if (!existsSync(docsDir)) {
    console.log("  ⚠ docs/ directory not found, skipping doc cards");
    return 0;
  }

  let count = 0;

  function scanDir(dir, prefix) {
    const entries = readdirSync(dir, { withFileTypes: true });
    for (const entry of entries) {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        // Skip api/ since those are generated from openapi
        if (entry.name === "api") continue;
        scanDir(fullPath, prefix ? `${prefix}/${entry.name}` : entry.name);
      } else if (entry.name.endsWith(".md") || entry.name.endsWith(".mdx")) {
        const slug = prefix
          ? `${prefix}/${entry.name.replace(/\.(md|mdx)$/, "")}`
          : entry.name.replace(/\.(md|mdx)$/, "");
        // Skip index files mapped to directory slug
        const finalSlug = slug.replace(/\/index$/, "");
        processDocFile(fullPath, finalSlug);
        count++;
      }
    }
  }

  function processDocFile(filePath, slug) {
    const content = readFileSync(filePath, "utf8");
    const fmMatch = content.match(/^---\n([\s\S]*?)\n---/);
    if (!fmMatch) return;
    const fm = fmMatch[1];
    const title = fm.match(/^title:\s*["']?(.+?)["']?\s*$/m)?.[1] || slug;
    const description = fm.match(/^description:\s*["']?(.+?)["']?\s*$/m)?.[1] || "";

    // Derive breadcrumb from slug path
    const parts = slug.split("/");
    const breadcrumb = parts.length > 1
      ? parts.slice(0, -1).map((p) => p.replace(/-/g, " ").replace(/\b\w/g, (c) => c.toUpperCase())).join(" › ")
      : "";

    const svg = docPageSvg(title, description, breadcrumb);
    const outPath = path.join(root, `public/og/${slug}.png`);
    svgToPng(svg, outPath);
  }

  scanDir(docsDir, "");
  return count;
}

// ─── Main ───────────────────────────────────────────────────────────────────

console.log("Generating OG images…");
await generateFallback();
const apiCount = await generateApiCards();
console.log(`  ✓ ${apiCount} API operation cards`);
const docCount = await generateDocCards();
console.log(`  ✓ ${docCount} doc page cards`);
console.log("Done.");
