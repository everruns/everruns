import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import yaml from "js-yaml";
import { marked } from "marked";
import { createHighlighter } from "shiki";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const docsAppRoot = path.resolve(scriptDir, "..");
const repoRoot = path.resolve(docsAppRoot, "../..");
const docsRoot = path.join(repoRoot, "docs");
const tutorialsRoot = path.join(docsRoot, "tutorials");
const generatedRoot = path.join(docsAppRoot, "src/generated/notebooks");
const publicRoot = path.join(docsAppRoot, "public/notebooks");

marked.setOptions({ gfm: true });

const highlighter = await createHighlighter({
  themes: ["github-light", "github-dark"],
  langs: [
    "python",
    "bash",
    "shellscript",
    "json",
    "yaml",
    "javascript",
    "typescript",
    "tsx",
    "jsx",
    "markdown",
    "text",
    "plaintext",
  ],
});

const loadedLanguages = new Set(highlighter.getLoadedLanguages());
const languageAliases = new Map([
  ["console", "bash"],
  ["py", "python"],
  ["python3", "python"],
  ["sh", "bash"],
  ["shell", "bash"],
  ["text/plain", "text"],
  ["zsh", "bash"],
]);

function parseFrontmatter(fileContent) {
  const match = fileContent.match(/^---\r?\n([\s\S]*?)\r?\n---\r?\n?/);
  if (!match) {
    return { data: {}, body: fileContent };
  }

  return {
    data: yaml.load(match[1]) ?? {},
    body: fileContent.slice(match[0].length),
  };
}

function escapeHtml(value) {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

function escapeMarkdownHtml(value) {
  return value.replaceAll("<", "&lt;").replaceAll(">", "&gt;");
}

const SAFE_MARKDOWN_PROTOCOLS = new Set(["http:", "https:", "mailto:", "tel:"]);

function sanitizeMarkdownUrl(value) {
  if (typeof value !== "string") {
    return null;
  }

  const trimmed = value.trim();
  if (trimmed.length === 0) {
    return null;
  }

  if (
    trimmed.startsWith("#") ||
    trimmed.startsWith("/") ||
    trimmed.startsWith("./") ||
    trimmed.startsWith("../")
  ) {
    return trimmed;
  }

  if (trimmed.startsWith("//")) {
    return trimmed;
  }

  try {
    const parsed = new URL(trimmed);
    return SAFE_MARKDOWN_PROTOCOLS.has(parsed.protocol.toLowerCase()) ? trimmed : null;
  } catch {
    return null;
  }
}

function stripHtmlTags(value) {
  return value.replace(/<[^>]+>/g, "");
}

function formatPublished(value) {
  if (typeof value !== "string" || value.length === 0) {
    return null;
  }

  const dateOnlyMatch = value.match(/^(\d{4})-(\d{2})-(\d{2})$/);
  const date = dateOnlyMatch
    ? new Date(Date.UTC(Number(dateOnlyMatch[1]), Number(dateOnlyMatch[2]) - 1, Number(dateOnlyMatch[3])))
    : new Date(value);
  if (Number.isNaN(date.getTime())) {
    return value;
  }

  return new Intl.DateTimeFormat("en-US", {
    dateStyle: "long",
    timeZone: "UTC",
  }).format(date);
}

function toText(value) {
  if (Array.isArray(value)) {
    return value.join("");
  }
  return value ?? "";
}

function normalizeStringList(value) {
  if (!Array.isArray(value)) {
    return [];
  }

  return value
    .filter((entry) => entry !== null && entry !== undefined)
    .map((entry) => String(entry).trim())
    .filter(Boolean);
}

function normalizeLanguage(value, fallback = "text") {
  const normalized = (value ?? fallback).toString().trim().toLowerCase();
  const aliased = languageAliases.get(normalized) ?? normalized;
  return loadedLanguages.has(aliased) ? aliased : fallback;
}

function slugifyHeading(value, slugCounts) {
  const base =
    stripHtmlTags(value)
      .normalize("NFKD")
      .replace(/[\u0300-\u036f]/g, "")
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-+|-+$/g, "") || "section";

  const nextCount = (slugCounts.get(base) ?? 0) + 1;
  slugCounts.set(base, nextCount);

  return nextCount === 1 ? base : `${base}-${nextCount}`;
}

function buildTocTree(headings) {
  const root = [];
  const stack = [];

  for (const heading of headings) {
    const node = { ...heading, children: [] };

    while (stack.length > 0 && stack.at(-1).depth >= node.depth) {
      stack.pop();
    }

    if (stack.length === 0) {
      root.push(node);
    } else {
      stack.at(-1).children.push(node);
    }

    stack.push(node);
  }

  return root;
}

function flattenTocDepths(items, depths = []) {
  for (const item of items) {
    depths.push(item.depth);
    flattenTocDepths(item.children ?? [], depths);
  }

  return depths;
}

function renderHighlightedCode(source, language) {
  return highlighter.codeToHtml(source, {
    lang: normalizeLanguage(language),
    themes: {
      light: "github-light",
      dark: "github-dark",
    },
    defaultColor: false,
  });
}

function createMarkdownRenderer({ slugCounts, headings, includeTocHeadings, assignHeadingIds }) {
  const renderer = new marked.Renderer();

  renderer.heading = function heading(token) {
    const content = this.parser.parseInline(token.tokens);
    const text = stripHtmlTags(token.text ?? "").trim();
    const shouldAssignId = assignHeadingIds && token.depth >= 2 && text.length > 0;
    const slug = shouldAssignId ? slugifyHeading(text, slugCounts) : null;

    if (includeTocHeadings && token.depth >= 2 && token.depth <= 3 && slug) {
      headings.push({
        depth: token.depth,
        slug,
        text,
      });
    }

    return slug
      ? `<h${token.depth} id="${escapeHtml(slug)}">${content}</h${token.depth}>`
      : `<h${token.depth}>${content}</h${token.depth}>`;
  };

  renderer.code = function code(token) {
    return `
      <div class="cookbook-markdown-code not-content">
        ${renderHighlightedCode(token.text, token.lang)}
      </div>
    `;
  };

  renderer.link = function link(token) {
    const href = sanitizeMarkdownUrl(token.href);
    const content = this.parser.parseInline(token.tokens);
    if (!href) {
      return content;
    }

    const title = token.title ? ` title="${escapeHtml(token.title)}"` : "";
    return `<a href="${escapeHtml(href)}"${title}>${content}</a>`;
  };

  renderer.image = function image(token) {
    const href = sanitizeMarkdownUrl(token.href);
    const alt = escapeHtml(token.text ?? "");
    if (!href) {
      return alt;
    }

    const title = token.title ? ` title="${escapeHtml(token.title)}"` : "";
    return `<img src="${escapeHtml(href)}" alt="${alt}"${title}>`;
  };

  return renderer;
}

function renderMarkdownHtml(source, options) {
  return marked.parse(escapeMarkdownHtml(source), {
    renderer: createMarkdownRenderer(options),
  });
}

function renderOutput(output) {
  if (output.output_type === "stream") {
    return { kind: "text", value: toText(output.text) };
  }

  if (output.output_type === "execute_result" || output.output_type === "display_data") {
    if (typeof output.data?.["image/png"] === "string") {
      return {
        kind: "image",
        value: `data:image/png;base64,${output.data["image/png"]}`,
      };
    }

    if (output.data?.["text/markdown"]) {
      return { kind: "markdown", value: toText(output.data["text/markdown"]) };
    }

    return { kind: "text", value: toText(output.data?.["text/plain"]) };
  }

  if (output.output_type === "error") {
    if (Array.isArray(output.traceback) && output.traceback.length > 0) {
      return { kind: "text", value: output.traceback.join("\n") };
    }
    return { kind: "text", value: [output.ename, output.evalue].filter(Boolean).join(": ") };
  }

  return null;
}

function toPosix(filePath) {
  return filePath.split(path.sep).join("/");
}

function deriveRoute(docPath, frontmatter) {
  if (typeof frontmatter.slug === "string" && frontmatter.slug.length > 0) {
    const slug = frontmatter.slug.replace(/^\/+|\/+$/g, "");
    return `/${slug}/`;
  }

  let relative = toPosix(path.relative(docsRoot, docPath));
  relative = relative.replace(/\.(md|mdx)$/, "");
  if (relative.endsWith("/index")) {
    relative = relative.slice(0, -"/index".length);
  }
  return `/${relative}/`;
}

function walkFiles(rootDir, predicate) {
  const files = [];

  function visit(currentDir) {
    for (const entry of readdirSync(currentDir, { withFileTypes: true })) {
      const fullPath = path.join(currentDir, entry.name);
      if (entry.isDirectory()) {
        visit(fullPath);
        continue;
      }

      if (predicate(fullPath)) {
        files.push(fullPath);
      }
    }
  }

  if (existsSync(rootDir)) {
    visit(rootDir);
  }

  return files;
}

function stripLeadingTitle(markdownSource) {
  const lines = markdownSource.split(/\r?\n/);
  const firstContentIndex = lines.findIndex((line) => line.trim().length > 0);
  if (firstContentIndex === -1 || !lines[firstContentIndex].startsWith("# ")) {
    return markdownSource;
  }

  lines.splice(firstContentIndex, 1);

  while (lines[firstContentIndex] !== undefined && lines[firstContentIndex].trim().length === 0) {
    lines.splice(firstContentIndex, 1);
  }

  return lines.join("\n");
}

function renderNotebookHtml({ notebook, sourcePath, publicHref, frontmatter }) {
  const language =
    notebook.metadata?.language_info?.name ?? notebook.metadata?.kernelspec?.language ?? "python";
  const kernelLabel = (
    notebook.metadata?.kernelspec?.display_name ??
    notebook.metadata?.language_info?.name ??
    "Python"
  ).toString();
  const published = formatPublished(frontmatter.published);
  const topics = normalizeStringList(frontmatter.topics);
  const sourceUrl =
    typeof frontmatter.github === "string" && frontmatter.github.length > 0
      ? frontmatter.github
      : `https://github.com/everruns/everruns/blob/main/${sourcePath}`;

  const metaBlock = `
    <section class="cookbook-meta not-content">
      ${
        topics.length > 0
          ? `
            <div class="cookbook-meta__topics">
              ${topics
                .map((topic) => `<span class="cookbook-meta__topic">${escapeHtml(topic)}</span>`)
                .join("")}
            </div>
          `
          : ""
      }
      <div class="cookbook-meta__row">
        <div class="cookbook-meta__aside">
          ${
            published
              ? `<p class="cookbook-meta__published">Published on ${escapeHtml(published)}</p>`
              : ""
          }
          <div class="cookbook-meta__actions">
            <a class="cookbook-meta__link" href="${escapeHtml(sourceUrl)}">View on GitHub</a>
            <a class="cookbook-meta__link" href="${escapeHtml(publicHref)}" download>Download .ipynb</a>
            <span class="cookbook-meta__kernel">${escapeHtml(kernelLabel)}</span>
          </div>
        </div>
      </div>
    </section>
  `;

  const slugCounts = new Map();
  const tocHeadings = [];

  const cells = (notebook.cells ?? [])
    .map((cell, index) => {
      const outputs = (cell.outputs ?? []).map(renderOutput).filter(Boolean);
      const source =
        cell.cell_type === "markdown" && index === 0
          ? stripLeadingTitle(toText(cell.source).trimEnd())
          : toText(cell.source).trimEnd();

      if (cell.cell_type === "markdown") {
        if (source.trim().length === 0) {
          return "";
        }

        return `
          <section class="cookbook-section sl-markdown-content">
            ${renderMarkdownHtml(source, {
              slugCounts,
              headings: tocHeadings,
              includeTocHeadings: true,
              assignHeadingIds: true,
            })}
          </section>
        `;
      }

      const renderedOutputs =
        outputs.length === 0
          ? ""
          : `
            <div class="cookbook-code__outputs">
              ${outputs.map((output) => {
                if (output.kind === "image") {
                  return `<img class="cookbook-code__image" src="${escapeHtml(output.value)}" alt="Notebook output" />`;
                }

                if (output.kind === "markdown") {
                  return `
                    <div class="cookbook-code__output cookbook-code__output--markdown sl-markdown-content">
                      ${renderMarkdownHtml(output.value, {
                        slugCounts,
                        headings: [],
                        includeTocHeadings: false,
                        assignHeadingIds: false,
                      })}
                    </div>
                  `;
                }

                return `
                  <div class="cookbook-code__output cookbook-code__output--text">
                    ${renderHighlightedCode(output.value.trim(), "text")}
                  </div>
                `;
              }).join("")}
            </div>
          `;

      return `
        <section class="cookbook-code not-content">
          <div class="cookbook-code__header">
            <span class="cookbook-code__language">${escapeHtml(language)}</span>
          </div>
          <div class="cookbook-code__body">
            <div class="cookbook-code__pre">
              ${renderHighlightedCode(source, language)}
            </div>
            ${renderedOutputs}
          </div>
        </section>
      `;
    })
    .join("");

  const toc = buildTocTree(tocHeadings);
  const tocDepths = flattenTocDepths(toc);

  return {
    html: `
      ${metaBlock}
      <article class="cookbook-article">
        ${cells}
      </article>
    `,
    toc: {
      items: toc,
      minHeadingLevel: tocDepths.length > 0 ? Math.min(...tocDepths) : 2,
      maxHeadingLevel: tocDepths.length > 0 ? Math.max(...tocDepths) : 3,
    },
  };
}

rmSync(generatedRoot, { recursive: true, force: true });
rmSync(publicRoot, { recursive: true, force: true });
mkdirSync(generatedRoot, { recursive: true });
mkdirSync(publicRoot, { recursive: true });

const docFiles = walkFiles(docsRoot, (filePath) => /\.(md|mdx)$/.test(filePath));
const tutorialNotebookFiles = walkFiles(tutorialsRoot, (filePath) => filePath.endsWith(".ipynb"));
const manifest = {};
const referencedNotebooks = new Map();

for (const docPath of docFiles) {
  const { data } = parseFrontmatter(readFileSync(docPath, "utf8"));
  if (typeof data.notebook !== "string" || data.notebook.length === 0) {
    continue;
  }

  const docRelativePath = toPosix(path.relative(repoRoot, docPath));
  if (!docPath.endsWith(".mdx") || path.basename(docPath) !== "index.mdx") {
    throw new Error(
      `Notebook-backed docs pages must use docs/**/index.mdx wrappers. Invalid wrapper: ${docRelativePath}`
    );
  }

  const notebookPath = path.resolve(path.dirname(docPath), data.notebook);
  if (!notebookPath.startsWith(`${docsRoot}${path.sep}`) || !notebookPath.endsWith(".ipynb")) {
    throw new Error(
      `Notebook frontmatter in ${docRelativePath} must point to a .ipynb file under docs/`
    );
  }

  if (!existsSync(notebookPath)) {
    throw new Error(`Notebook file not found: ${toPosix(path.relative(repoRoot, notebookPath))}`);
  }

  const notebookKey = toPosix(path.relative(repoRoot, notebookPath));
  if (referencedNotebooks.has(notebookKey)) {
    throw new Error(
      `Notebook ${notebookKey} is referenced by multiple docs pages: ${referencedNotebooks.get(
        notebookKey
      )} and ${docRelativePath}`
    );
  }

  referencedNotebooks.set(notebookKey, docRelativePath);

  const notebook = JSON.parse(readFileSync(notebookPath, "utf8"));
  const notebookRelativeToDocs = toPosix(path.relative(docsRoot, notebookPath));
  const htmlRelativePath = notebookRelativeToDocs.replace(/\.ipynb$/, ".html");
  const htmlOutputPath = path.join(generatedRoot, htmlRelativePath);
  const publicNotebookPath = path.join(publicRoot, notebookRelativeToDocs);
  const publicHref = `/notebooks/${notebookRelativeToDocs}`;

  mkdirSync(path.dirname(htmlOutputPath), { recursive: true });
  mkdirSync(path.dirname(publicNotebookPath), { recursive: true });
  copyFileSync(notebookPath, publicNotebookPath);

  const renderedNotebook = renderNotebookHtml({
    notebook,
    sourcePath: notebookKey,
    publicHref,
    frontmatter: data,
  });

  writeFileSync(htmlOutputPath, renderedNotebook.html, "utf8");

  manifest[notebookKey] = {
    htmlFile: toPosix(htmlRelativePath),
    publicHref,
    route: deriveRoute(docPath, data),
    docPage: docRelativePath,
    notebookFile: notebookKey,
    toc: renderedNotebook.toc,
  };
}

const unreferencedNotebooks = tutorialNotebookFiles
  .map((filePath) => toPosix(path.relative(repoRoot, filePath)))
  .filter((filePath) => !referencedNotebooks.has(filePath));

if (unreferencedNotebooks.length > 0) {
  throw new Error(
    `Each docs/tutorials notebook must be referenced by exactly one MDX wrapper via frontmatter notebook:. Missing references for:\n${unreferencedNotebooks.join(
      "\n"
    )}`
  );
}

writeFileSync(path.join(generatedRoot, "manifest.json"), `${JSON.stringify(manifest, null, 2)}\n`, "utf8");

console.log(`Rendered ${Object.keys(manifest).length} notebook-backed docs page(s).`);
