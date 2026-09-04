import { readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { load, JSON_SCHEMA } from "js-yaml";

const WIDTH = 2400;
const HEIGHT = 1350;
const MONTHS = ["JAN", "FEB", "MAR", "APR", "MAY", "JUN", "JUL", "AUG", "SEP", "OCT", "NOV", "DEC"];

function fail(message) {
  throw new Error(`Invalid release card: ${message}`);
}

function text(value, field, maxLength) {
  if (typeof value !== "string" || value.trim() === "") fail(`${field} must be a non-empty string`);
  const result = value.trim();
  if (result.length > maxLength) fail(`${field} must be at most ${maxLength} characters`);
  return result;
}

function lines(value, field, { min, max, lineLength }) {
  if (!Array.isArray(value) || value.length < min || value.length > max) {
    fail(`${field} must contain ${min}-${max} lines`);
  }
  return value.map((line, index) => text(line, `${field}[${index}]`, lineLength));
}

function isCalendarDate(value) {
  if (!/^\d{4}-\d{2}-\d{2}$/.test(value)) return false;
  const [year, month, day] = value.split("-").map(Number);
  const parsed = new Date(Date.UTC(year, month - 1, day));
  return parsed.getUTCFullYear() === year && parsed.getUTCMonth() === month - 1 && parsed.getUTCDate() === day;
}

export function validateReleaseCard(value, expectedVersion) {
  if (!value || typeof value !== "object" || Array.isArray(value)) fail("document must be an object");

  const version = text(value.version, "version", 24);
  if (!/^\d+\.\d+\.\d+$/.test(version)) fail("version must be semantic version X.Y.Z");
  if (expectedVersion && version !== expectedVersion) {
    fail(`version ${version} does not match release ${expectedVersion}`);
  }

  const date = text(value.date, "date", 10);
  if (!isCalendarDate(date)) {
    fail("date must be a valid YYYY-MM-DD date");
  }

  const headline = lines(value.headline, "headline", { min: 1, max: 2, lineLength: 24 });
  const summary = lines(value.summary, "summary", { min: 1, max: 3, lineLength: 48 });

  if (!Array.isArray(value.highlights) || value.highlights.length < 1 || value.highlights.length > 3) {
    fail("highlights must contain 1-3 items");
  }
  const highlights = value.highlights.map((item, index) => {
    if (!item || typeof item !== "object" || Array.isArray(item)) fail(`highlights[${index}] must be an object`);
    const highlight = {
      title: text(item.title, `highlights[${index}].title`, 36),
      description: text(item.description, `highlights[${index}].description`, 100),
      source: text(item.source, `highlights[${index}].source`, 80),
    };
    if (wrap(highlight.description, 47).length > 2) {
      fail(`highlights[${index}].description must fit on two lines`);
    }
    return highlight;
  });

  return { version, date, headline, summary, highlights };
}

export function validateChangelog(card, changelog) {
  const heading = `## [${card.version}] - ${card.date}`;
  const start = changelog.indexOf(heading);
  if (start === -1) fail(`CHANGELOG.md is missing ${heading}`);
  const next = changelog.indexOf("\n## [", start + heading.length);
  const section = changelog.slice(start, next === -1 ? undefined : next);
  for (const [index, highlight] of card.highlights.entries()) {
    if (!section.includes(highlight.source)) {
      fail(`highlights[${index}].source ${highlight.source} is not present in the v${card.version} changelog`);
    }
  }
}

export function readReleaseCard(inputPath, expectedVersion) {
  const parsed = load(readFileSync(inputPath, "utf8"), { schema: JSON_SCHEMA });
  return validateReleaseCard(parsed, expectedVersion);
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
  const words = value.split(/\s+/);
  const result = [];
  let current = "";
  for (const word of words) {
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

function releaseLabel(card) {
  const month = MONTHS[Number(card.date.slice(5, 7)) - 1];
  return `RELEASE  ·  ${month} ${card.date.slice(0, 4)}  ·  V${card.version}`;
}

export function releaseCardSvg(card) {
  const headline = card.headline
    .map((line, index) => `<text x="210" y="${535 + index * 170}" class="headline">${escapeXml(line)}</text>`)
    .join("\n    ");
  const summaryStart = 790 + Math.max(0, card.headline.length - 2) * 40;
  const summary = card.summary
    .map((line, index) => `<text x="210" y="${summaryStart + index * 68}" class="summary">${escapeXml(line)}</text>`)
    .join("\n    ");

  const itemGap = card.highlights.length === 1 ? 0 : 250;
  const itemsStart = card.highlights.length === 1 ? 570 : card.highlights.length === 2 ? 465 : 395;
  const highlights = card.highlights
    .map((item, index) => {
      const y = itemsStart + index * itemGap;
      const descriptionLines = wrap(item.description, 47).slice(0, 2);
      return `<g transform="translate(1415 ${y})">
      <text x="0" y="34" class="number">${String(index + 1).padStart(2, "0")}</text>
      <line x1="98" y1="-16" x2="98" y2="155" class="item-rule"/>
      <text x="155" y="34" class="item-title">${escapeXml(item.title)}</text>
      ${descriptionLines.map((line, lineIndex) => `<text x="155" y="${91 + lineIndex * 48}" class="item-copy">${escapeXml(line)}</text>`).join("\n      ")}
    </g>`;
    })
    .join("\n    ");

  return `<svg xmlns="http://www.w3.org/2000/svg" width="${WIDTH}" height="${HEIGHT}" viewBox="0 0 ${WIDTH} ${HEIGHT}" role="img" aria-label="Everruns release ${escapeXml(card.version)}">
  <defs>
    <radialGradient id="wash" cx="0" cy="0" r="1" gradientTransform="translate(2070 80) rotate(131) scale(1050 820)" gradientUnits="userSpaceOnUse">
      <stop stop-color="#10263A" stop-opacity=".55"/>
      <stop offset="1" stop-color="#07101A" stop-opacity="0"/>
    </radialGradient>
    <filter id="soft" x="-20%" y="-20%" width="140%" height="140%">
      <feGaussianBlur stdDeviation="24"/>
    </filter>
    <style>
      .sans { font-family: Arial, Helvetica, sans-serif; }
      .mono { font-family: Menlo, Consolas, monospace; }
      .headline { font-family: Arial, Helvetica, sans-serif; font-size: 152px; font-weight: 700; letter-spacing: -7px; fill: #F4F3EF; }
      .summary { font-family: Arial, Helvetica, sans-serif; font-size: 46px; font-weight: 400; letter-spacing: -1px; fill: #A8B0BE; }
      .number { font-family: Arial, Helvetica, sans-serif; font-size: 42px; font-weight: 400; fill: #D4A43A; }
      .item-title { font-family: Arial, Helvetica, sans-serif; font-size: 41px; font-weight: 700; letter-spacing: -1px; fill: #F4F3EF; }
      .item-copy { font-family: Arial, Helvetica, sans-serif; font-size: 34px; font-weight: 400; fill: #A8B0BE; }
      .item-rule { stroke: #D4A43A; stroke-width: 4; }
    </style>
  </defs>
  <rect width="${WIDTH}" height="${HEIGHT}" fill="#07101A"/>
  <rect width="${WIDTH}" height="${HEIGHT}" fill="url(#wash)"/>
  <ellipse cx="580" cy="600" rx="620" ry="500" fill="#000812" opacity=".22" filter="url(#soft)"/>

  <text x="115" y="135" class="sans" font-size="54" font-weight="700" letter-spacing="-2" fill="#F4F3EF">Everruns</text>
  <text x="2285" y="127" class="mono" font-size="29" font-weight="700" letter-spacing="4" text-anchor="end" fill="#D4A43A">${escapeXml(releaseLabel(card))}</text>

  <line x1="116" y1="378" x2="116" y2="1000" stroke="#D4A43A" stroke-width="6"/>
  ${headline}
  ${summary}

  ${highlights}

  <line x1="115" y1="1186" x2="2285" y2="1186" stroke="#D4A43A" stroke-width="2"/>
  <text x="115" y="1260" class="mono" font-size="29" fill="#8F99A8">docs.everruns.com</text>
  <text x="2285" y="1260" class="mono" font-size="29" text-anchor="end" fill="#8F99A8">github.com/everruns/everruns</text>
</svg>`;
}

function parseArgs(argv) {
  const args = {
    input: "../../release-card.yml",
    changelog: "../../CHANGELOG.md",
    output: "../../release-card.png",
    check: false,
    svgOutput: undefined,
    expectedVersion: undefined,
  };
  for (let index = 0; index < argv.length; index += 1) {
    const option = argv[index];
    if (option === "--check") args.check = true;
    else if (option === "--input") args.input = argv[++index];
    else if (option === "--changelog") args.changelog = argv[++index];
    else if (option === "--output") args.output = argv[++index];
    else if (option === "--svg-output") args.svgOutput = argv[++index];
    else if (option === "--expect-version") args.expectedVersion = argv[++index];
    else fail(`unknown argument ${option}`);
  }
  if (!args.input) fail("--input requires a path");
  if (!args.check && !args.output) fail("--output requires a path");
  return args;
}

async function main() {
  const args = parseArgs(process.argv.slice(2));
  const input = path.resolve(args.input);
  const card = readReleaseCard(input, args.expectedVersion);
  validateChangelog(card, readFileSync(path.resolve(args.changelog), "utf8"));
  if (args.check) {
    console.log(`release card v${card.version} is valid`);
    return;
  }

  const svg = releaseCardSvg(card);
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
