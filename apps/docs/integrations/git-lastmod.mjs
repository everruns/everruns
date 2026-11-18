// When each docs page's content last changed, from git, for the sitemap's
// <lastmod> (sitemap-enhance.mjs).
//
// Decision (user, 2026-11-17): <lastmod> is the date a page's content actually
// last changed, never the build date. A sitemap that stamps every page with
// the deploy date tells crawlers nothing, so they learn to ignore it.
//
// A page's sources are its file under docs/ (`<path>.md(x)` or
// `<path>/index.md(x)`). An index that is a leaf bundle (its directory holds no
// other pages: changelog entries, notebook wrappers) also counts the files
// beside it, so a changed screenshot or notebook dates the page. Generated
// API operation pages take the date of docs/api/openapi.json, and the
// changelog index and product views the newest changelog change. A page with no
// source file gets no <lastmod>.
//
// Git history must be complete: a shallow clone reports its oldest commit as
// touching every file, which would make old pages look new. On a shallow
// clone this returns no dates at all (docs-deploy checks out full history).
import { execFileSync } from "node:child_process";
import { existsSync, readdirSync } from "node:fs";
import { join, relative } from "node:path";

const PAGE = /\.mdx?$/;

// Parses `git log --format=@%cs --name-only` (newest first) into
// repo-relative path -> YYYY-MM-DD of its newest commit.
export function parseGitLog(log) {
  const dates = new Map();
  let date = null;
  for (const line of log.split("\n")) {
    if (line.startsWith("@")) date = line.slice(1);
    else if (line && date && !dates.has(line)) dates.set(line, date);
  }
  return dates;
}

export function gitFileDates(repoRoot, pathspec) {
  const git = (...args) =>
    execFileSync("git", args, {
      cwd: repoRoot,
      encoding: "utf8",
      maxBuffer: 256 * 1024 * 1024,
      stdio: ["ignore", "pipe", "pipe"],
    });
  if (git("rev-parse", "--is-shallow-repository").trim() === "true") {
    return null;
  }
  const dates = parseGitLog(
    git("log", "--format=@%cs", "--name-only", "--no-renames", "HEAD", "--", pathspec),
  );
  // Uncommitted edits (local builds) count as changed today.
  const today = new Date().toISOString().slice(0, 10);
  for (const line of git("status", "--porcelain", "--", pathspec).split("\n")) {
    const path = line.slice(3).trim();
    if (path) dates.set(path, today);
  }
  return dates;
}

function filesIn(dir) {
  return readdirSync(dir, { withFileTypes: true }).flatMap((entry) => {
    const path = join(dir, entry.name);
    return entry.isDirectory() ? filesIn(path) : [path];
  });
}

// Absolute source files for a docs page path relative to the docs base,
// e.g. "framework/quickstart" or "" for the docs home.
export function pageSources(contentDir, pagePath) {
  // The changelog index and its per-product views (changelog/framework/ ...)
  // list entries, so any entry change changes them.
  if (/^changelog(\/[a-z-]+)?$/.test(pagePath)) {
    return filesIn(join(contentDir, "changelog"));
  }
  const base = join(contentDir, pagePath);
  for (const ext of [".md", ".mdx"]) {
    if (pagePath && existsSync(base + ext)) return [base + ext];
  }
  for (const ext of [".md", ".mdx"]) {
    const index = join(base, `index${ext}`);
    if (!existsSync(index)) continue;
    const siblings = filesIn(base).filter((file) => file !== index);
    const leafBundle = !siblings.some((file) => PAGE.test(file));
    return leafBundle ? [index, ...siblings] : [index];
  }
  if (pagePath === "api" || pagePath.startsWith("api/")) {
    const spec = join(contentDir, "api/openapi.json");
    if (existsSync(spec)) return [spec];
  }
  return [];
}

// Newest date across files, or undefined.
export function newestDate(files, dates, repoRoot) {
  let best;
  for (const file of files) {
    const date = dates.get(relative(repoRoot, file));
    if (date && (!best || date > best)) best = date;
  }
  return best;
}

// Returns pagePath -> date resolver, or null when git cannot date pages.
export function docsLastmod({ repoRoot, contentDir }) {
  let dates;
  try {
    dates = gitFileDates(repoRoot, relative(repoRoot, contentDir));
  } catch (error) {
    console.warn(`[git-lastmod] git unavailable: ${error.message}`);
    return null;
  }
  if (!dates) {
    console.warn("[git-lastmod] shallow clone: sitemap omits <lastmod>");
    return null;
  }
  return (pagePath) =>
    newestDate(pageSources(contentDir, pagePath), dates, repoRoot);
}
