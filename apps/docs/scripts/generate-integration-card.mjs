// Integration card for a changelog entry that announces a new integration:
// the host product's mark (Everruns, Yolop, Bashkit or Mira), a plus, and the
// partner's logo, over the entry's title.
//
// Decision: same canvas, palette and type as the release card
// (generate-release-card.mjs), so integration and release visuals read as one
// family. Logos sit on white tiles because brand marks are designed for light
// backgrounds. A partner without a logo file renders as its name in a
// wordmark tile instead of blocking the entry.
//
//   node scripts/generate-integration-card.mjs --host everruns --partner slack \
//     --title "Everruns agents run in Slack's agent pane" --date 2026-09-15 \
//     --output ../../docs/changelog/2026-09-15-slack-agent-pane/integration@2x.png
//
// --partner repeats for an entry that adds several integrations at once
// (at most three). --name overrides a partner's display name, in order.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const WIDTH = 2400;
const HEIGHT = 1350;
const MONTHS = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];
const HOSTS = { everruns: "Everruns", yolop: "Yolop", bashkit: "Bashkit", mira: "Mira" };
const ASSETS = path.join(path.dirname(fileURLToPath(import.meta.url)), "../src/assets/changelog");

function fail(message) {
  throw new Error(`Invalid integration card: ${message}`);
}

function escapeXml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&apos;");
}

function wrap(value, maxCharacters) {
  const result = [];
  let current = "";
  for (const word of value.split(/\s+/)) {
    if (current && `${current} ${word}`.length > maxCharacters) {
      result.push(current);
      current = word;
    } else {
      current = current ? `${current} ${word}` : word;
    }
  }
  if (current) result.push(current);
  return result;
}

// Inlines a logo file as a nested <svg> placed in a size x size box. Ids are
// prefixed so two logos with the same gradient id cannot collide.
export function nestedLogo(source, x, y, size, prefix) {
  const viewBox = source.match(/viewBox="([^"]+)"/)?.[1];
  if (!viewBox) fail("logo has no viewBox");
  const inner = source
    .replace(/^[\s\S]*?<svg[^>]*>/, "")
    .replace(/<\/svg>\s*$/, "")
    .replace(/<title>[\s\S]*?<\/title>/g, "")
    .replaceAll(/id="([^"]+)"/g, `id="${prefix}-$1"`)
    .replaceAll(/url\(#([^)]+)\)/g, `url(#${prefix}-$1)`);
  const rootFill = source.match(/<svg[^>]*\sfill="([^"]+)"/)?.[1];
  return `<svg x="${x}" y="${y}" width="${size}" height="${size}" viewBox="${viewBox}"${rootFill ? ` fill="${rootFill}"` : ""}>${inner}</svg>`;
}

function tile(item, x, y, size, prefix) {
  const body = item.logo
    ? nestedLogo(item.logo, x + size * 0.2, y + size * 0.2, size * 0.6, prefix)
    : `<text x="${x + size / 2}" y="${y + size / 2 + 26}" text-anchor="middle" class="wordmark" font-size="${Math.min(76, Math.round((size * 0.78) / (item.name.length * 0.56)))}">${escapeXml(item.name)}</text>`;
  return `<rect x="${x}" y="${y}" width="${size}" height="${size}" rx="44" fill="#F4F3EF"/>
  ${body}
  <text x="${x + size / 2}" y="${y + size + 82}" text-anchor="middle" class="label">${escapeXml(item.name)}</text>`;
}

// host and each partner are { name, logo? }, logo being SVG source.
export function integrationCardSvg({ host, partners, title, date }) {
  const items = [host, ...partners];
  const size = items.length > 3 ? 300 : 360;
  const gap = 150;
  const total = items.length * size + (items.length - 1) * gap;
  let x = (WIDTH - total) / 2;
  const y = 330;
  const parts = [];
  items.forEach((item, index) => {
    if (index > 0) {
      const cx = x - gap / 2;
      const glyph = index === 1 ? "+" : "&amp;";
      parts.push(`<text x="${cx}" y="${y + size / 2 + 38}" text-anchor="middle" class="join">${glyph}</text>`);
    }
    parts.push(tile(item, x, y, size, `l${index}`));
    x += size + gap;
  });
  const lines = wrap(title, 44).slice(0, 2);
  const titleSvg = lines
    .map((line, index) => `<text x="${WIDTH / 2}" y="${1000 + index * 92}" text-anchor="middle" class="title">${escapeXml(line)}</text>`)
    .join("\n  ");
  const month = MONTHS[Number(date.slice(5, 7)) - 1];
  return `<svg xmlns="http://www.w3.org/2000/svg" width="${WIDTH}" height="${HEIGHT}" viewBox="0 0 ${WIDTH} ${HEIGHT}" role="img" aria-label="${escapeXml(`${items.map((i) => i.name).join(" and ")} integration`)}">
  <defs>
    <radialGradient id="wash" cx="0" cy="0" r="1" gradientTransform="translate(2070 80) rotate(131) scale(1050 820)" gradientUnits="userSpaceOnUse">
      <stop stop-color="#10263A" stop-opacity=".55"/>
      <stop offset="1" stop-color="#07101A" stop-opacity="0"/>
    </radialGradient>
    <style>
      .title { font-family: Arial, Helvetica, sans-serif; font-size: 76px; font-weight: 700; letter-spacing: -2px; fill: #F4F3EF; }
      .label { font-family: Arial, Helvetica, sans-serif; font-size: 40px; font-weight: 400; fill: #A8B0BE; }
      .wordmark { font-family: Arial, Helvetica, sans-serif; font-weight: 700; letter-spacing: -2px; fill: #07101A; }
      .join { font-family: Arial, Helvetica, sans-serif; font-size: 120px; font-weight: 400; fill: #D4A43A; }
      .mono { font-family: Menlo, Consolas, monospace; }
    </style>
  </defs>
  <rect width="${WIDTH}" height="${HEIGHT}" fill="#07101A"/>
  <rect width="${WIDTH}" height="${HEIGHT}" fill="url(#wash)"/>
  <text x="115" y="135" font-family="Arial, Helvetica, sans-serif" font-size="54" font-weight="700" letter-spacing="-2" fill="#F4F3EF">Everruns</text>
  <text x="2285" y="127" class="mono" font-size="29" font-weight="700" letter-spacing="4" text-anchor="end" fill="#D4A43A">INTEGRATION  ·  ${month} ${date.slice(0, 4)}</text>
  ${parts.join("\n  ")}
  ${titleSvg}
  <line x1="115" y1="1186" x2="2285" y2="1186" stroke="#D4A43A" stroke-width="2"/>
  <text x="115" y="1260" class="mono" font-size="29" fill="#8F99A8">docs.everruns.com/changelog</text>
</svg>`;
}

function parseArgs(argv) {
  const args = { partners: [], names: [] };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    const value = argv[++index];
    if (option === "--host") args.host = value;
    else if (option === "--partner") args.partners.push(value);
    else if (option === "--name") args.names.push(value);
    else if (option === "--title") args.title = value;
    else if (option === "--date") args.date = value;
    else if (option === "--output") args.output = value;
    else if (option === "--svg-output") args.svgOutput = value;
    else fail(`unknown argument ${option}`);
  }
  if (!HOSTS[args.host]) fail(`--host must be one of ${Object.keys(HOSTS).join(", ")}`);
  if (args.partners.length < 1 || args.partners.length > 3) fail("--partner must be given 1-3 times");
  if (!args.title || args.title.length > 88) fail("--title must be 1-88 characters");
  if (!/^\d{4}-\d{2}-\d{2}$/.test(args.date ?? "")) fail("--date must be YYYY-MM-DD");
  if (!args.output) fail("--output requires a path");
  return args;
}

function readLogo(file) {
  return existsSync(file) ? readFileSync(file, "utf8") : undefined;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const host = { name: HOSTS[args.host], logo: readLogo(path.join(ASSETS, "products", `${args.host}.svg`)) };
  const partners = args.partners.map((slug, index) => ({
    name: args.names[index] ?? slug,
    logo: readLogo(path.join(ASSETS, "logos", `${slug}.svg`)),
  }));
  const svg = integrationCardSvg({ host, partners, title: args.title, date: args.date });
  if (args.svgOutput) writeFileSync(path.resolve(args.svgOutput), svg);
  const { default: sharp } = await import("sharp");
  const output = path.resolve(args.output);
  await sharp(Buffer.from(svg)).png().toFile(output);
  console.log(`generated ${output}`);
}

const isCli = process.argv[1] && fileURLToPath(import.meta.url) === path.resolve(process.argv[1]);
if (isCli) {
  main().catch((error) => {
    console.error(error.message);
    process.exitCode = 1;
  });
}
