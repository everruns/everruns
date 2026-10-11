// Validates the public changelog (docs/changelog/) before the docs build.
//
// The content schema (src/content.config.ts) types the fields; this script
// holds the rules that make an entry an entry, from knowledge/docs/changelog.md:
// one folder per entry with its media beside it, required fields, title and
// description lengths for search results, a compact body, no marketing words,
// real docs links, and screenshots captured at 2x so they stay sharp.
//
// Usage: node scripts/check-changelog.mjs   (exit 1 on any error)
import { existsSync, readdirSync, readFileSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

import { JSON_SCHEMA, load } from "js-yaml";
import sharp from "sharp";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const docsDir = path.resolve(__dirname, "../../../docs");
const changelogDir = path.join(docsDir, "changelog");

const KINDS = ["feature", "improvement", "fix", "breaking", "deprecation", "incident", "release"];
const SIGNIFICANCE = ["major", "notable", "minor"];
const PRODUCTS = ["cloud", "platform", "framework", "sdk", "cli", "yolop", "mira", "bashkit"];
const SURFACES = ["ui", "api", "code", "cli", "agent", "operations"];
// Products whose docs live outside docs.everruns.com; their entries may link
// their own repository instead of an internal docs page.
const EXTERNAL_DOCS = ["yolop", "mira", "bashkit", "sdk"];
const NEEDS_DOCS = ["feature", "improvement", "breaking", "deprecation"];
const BANNED = [
  "excited", "thrilled", "powerful", "seamless", "seamlessly", "game-changing", "game changer",
  "revolutionary", "effortless", "effortlessly", "robust", "cutting-edge", "best-in-class",
  "supercharge", "unleash", "delightful", "blazing", "world-class",
];
const PAGES = new Set(["index.mdx", "feeds.md"]);
const RASTER = /\.(png|webp)$/i;
const VIDEO = /\.(mp4|webm)$/i;
const MAX_VIDEO_BYTES = 20 * 1024 * 1024;

const errors = [];
const warnings = [];
const fail = (where, message) => errors.push(`${where}: ${message}`);

function docsPageExists(link) {
  const clean = link.split("#")[0].replace(/^\/|\/$/g, "");
  if (clean === "") return true;
  return [".md", ".mdx", "/index.md", "/index.mdx"].some((ext) =>
    existsSync(path.join(docsDir, clean + ext)),
  );
}

function words(markdown) {
  return markdown
    .replace(/```[\s\S]*?```/g, " ")
    .replace(/<[^>]+>/g, " ")
    .replace(/!\[[^\]]*\]\([^)]*\)/g, " ")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .split(/\s+/)
    .filter((w) => /[A-Za-z0-9]/.test(w)).length;
}

async function checkEntry(folder) {
  const dir = path.join(changelogDir, folder);
  const where = `changelog/${folder}`;
  const indexPath = path.join(dir, "index.md");
  if (!/^\d{4}-\d{2}-\d{2}-[a-z0-9]+(-[a-z0-9]+)*$/.test(folder)) {
    fail(where, "folder must be named YYYY-MM-DD-lowercase-slug");
  }
  if (!existsSync(indexPath)) return fail(where, "missing index.md");

  const source = readFileSync(indexPath, "utf8");
  const match = source.match(/^---\n([\s\S]*?)\n---\n?([\s\S]*)$/);
  if (!match) return fail(where, "missing frontmatter");
  let fm;
  try {
    fm = load(match[1], { schema: JSON_SCHEMA }) ?? {};
  } catch (error) {
    return fail(where, `invalid frontmatter: ${error.message}`);
  }
  const body = match[2];
  const isRelease = fm.kind === "release";

  const title = String(fm.title ?? "");
  const [minTitle, maxTitle] = isRelease ? [10, 70] : [30, 70];
  if (title.length < minTitle || title.length > maxTitle) {
    fail(where, `title is ${title.length} characters; keep it ${minTitle}-${maxTitle} for search results`);
  }
  const description = String(fm.description ?? "");
  if (description.length < 120 || description.length > 160) {
    fail(where, `description is ${description.length} characters; keep it 120-160 (meta description)`);
  }
  if (String(fm.date ?? "") !== folder.slice(0, 10)) {
    fail(where, `date ${fm.date} must match the folder's date ${folder.slice(0, 10)}`);
  }
  if (!KINDS.includes(fm.kind)) fail(where, `kind must be one of ${KINDS.join(", ")}`);
  if (!SIGNIFICANCE.includes(fm.significance)) {
    fail(where, `significance must be one of ${SIGNIFICANCE.join(", ")}`);
  }
  const products = fm.products ?? [];
  if (!Array.isArray(products) || products.length === 0) fail(where, "products must list at least one product");
  for (const p of products) if (!PRODUCTS.includes(p)) fail(where, `unknown product ${p}`);
  const surfaces = fm.surfaces ?? [];
  if (!isRelease && fm.kind !== "incident" && surfaces.length === 0) {
    fail(where, "surfaces must list at least one surface");
  }
  for (const s of surfaces) if (!SURFACES.includes(s)) fail(where, `unknown surface ${s}`);
  if (fm.release !== undefined && !/^\d+\.\d+\.\d+$/.test(String(fm.release))) {
    fail(where, `release ${fm.release} must be a X.Y.Z version`);
  }

  const docs = fm.links?.docs ?? [];
  const external = products.length > 0 && products.every((p) => EXTERNAL_DOCS.includes(p));
  for (const link of docs) {
    if (external && link.startsWith("https://github.com/everruns/")) continue;
    if (!link.startsWith("/")) fail(where, `links.docs ${link} must be a site path like /features/chat/`);
    else if (!docsPageExists(link)) fail(where, `links.docs ${link} has no page in docs/`);
  }
  for (const pr of fm.links?.prs ?? []) {
    if (!/^everruns\/[\w.-]+#\d+$/.test(pr)) fail(where, `links.prs ${pr} must look like everruns/everruns#123`);
    if (pr.startsWith("everruns/saas#")) fail(where, "saas is private: do not link its pull requests");
  }
  if (NEEDS_DOCS.includes(fm.kind) && docs.length === 0 && !external) {
    fail(where, `a ${fm.kind} entry needs at least one links.docs page`);
  }

  if (fm.supersededBy && !existsSync(path.join(changelogDir, fm.supersededBy, "index.md"))) {
    fail(where, `supersededBy ${fm.supersededBy} is not an entry folder`);
  }

  const count = words(body);
  const [minWords, maxWords] = isRelease ? [20, 400] : [40, 150];
  if (count < minWords || count > maxWords) {
    fail(where, `body is ${count} words; keep it ${minWords}-${maxWords}`);
  }
  const prose = `${title}\n${description}\n${body.replace(/```[\s\S]*?```/g, "")}`;
  if (prose.includes("\u2014")) fail(where, "no em-dashes; use a comma, colon or a new sentence");
  if (/!(\s|$)/.test(prose.replace(/!\[/g, ""))) fail(where, "no exclamation marks");
  for (const word of BANNED) {
    if (new RegExp(`\\b${word}\\b`, "i").test(prose)) fail(where, `marketing word "${word}"`);
  }

  // Media: everything sits in the entry folder, is referenced by the entry,
  // and raster screenshots are 2x captures named name@2x.png.
  const referenced = new Set(
    [...body.matchAll(/(?:\]\(|src="|poster=")\.\/([^)"\s]+)/g)].map((m) => decodeURI(m[1])),
  );
  for (const ref of referenced) {
    if (!existsSync(path.join(dir, ref))) fail(where, `references ./${ref}, which is not in the entry folder`);
  }
  if (/\]\((?:\.\.\/|\/)[^)]*\.(png|jpe?g|webp|gif|mp4|webm)\)/i.test(body)) {
    fail(where, "media must sit in the entry's own folder and be referenced as ./file");
  }
  for (const file of readdirSync(dir)) {
    if (file === "index.md") continue;
    const full = path.join(dir, file);
    if (!referenced.has(file)) fail(where, `${file} is not referenced by index.md`);
    if (/\.(jpe?g|gif)$/i.test(file)) {
      fail(where, `${file}: use PNG for screenshots (lossless) and MP4 for motion, not JPEG or GIF`);
    } else if (RASTER.test(file)) {
      const { width = 0 } = await sharp(full).metadata();
      if (!/@2x\.(png|webp)$/i.test(file)) {
        fail(where, `${file}: screenshots are captured at 2x device scale; name them name@2x.png`);
      }
      if (width < 1000 && !/mobile/i.test(file)) {
        fail(where, `${file} is ${width}px wide; capture at 2x so it is at least 1000px wide`);
      }
      if (!new RegExp(`!\\[[^\\]]{8,}\\]\\(\\./${file.replace(/[.@]/g, "\\$&")}\\)`).test(body)
        && !file.includes("-poster")) {
        fail(where, `${file} needs descriptive alt text (8+ characters)`);
      }
    } else if (VIDEO.test(file)) {
      if (statSync(full).size > MAX_VIDEO_BYTES) fail(where, `${file} is over 20 MB`);
      if (!new RegExp(`poster="\\./[^"]+"[^>]*>|<video[^>]*poster=`).test(body)) {
        fail(where, `${file}: give the <video> a poster frame (name-poster@2x.png)`);
      }
    } else if (!/\.svg$/i.test(file)) {
      fail(where, `${file}: unsupported file in an entry folder`);
    }
  }

  if (fm.visual) warnings.push(`${where}: screenshot still to capture: ${fm.visual}`);
}

if (existsSync(changelogDir)) {
  for (const name of readdirSync(changelogDir).sort()) {
    const full = path.join(changelogDir, name);
    if (statSync(full).isDirectory()) await checkEntry(name);
    else if (!PAGES.has(name)) {
      fail(`changelog/${name}`, "entries are folders: changelog/YYYY-MM-DD-slug/index.md");
    }
  }
}

for (const warning of warnings) console.warn(`warn  ${warning}`);
if (errors.length > 0) {
  for (const error of errors) console.error(`error ${error}`);
  console.error(`\nChangelog check failed with ${errors.length} error(s). Rules: knowledge/docs/changelog.md`);
  process.exit(1);
}
console.log(`Changelog check passed (${readdirSync(changelogDir).length - PAGES.size} entries).`);
