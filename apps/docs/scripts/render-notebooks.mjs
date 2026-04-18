import {
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  rmSync,
  statSync,
  writeFileSync,
} from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import yaml from "js-yaml";
import { marked } from "marked";

const scriptDir = path.dirname(fileURLToPath(import.meta.url));
const docsAppRoot = path.resolve(scriptDir, "..");
const repoRoot = path.resolve(docsAppRoot, "../..");
const docsRoot = path.join(repoRoot, "docs");
const tutorialsRoot = path.join(docsRoot, "tutorials");
const generatedRoot = path.join(docsAppRoot, "src/generated/notebooks");
const publicRoot = path.join(docsAppRoot, "public/notebooks");

marked.setOptions({ gfm: true });

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

function toText(value) {
  if (Array.isArray(value)) {
    return value.join("");
  }
  return value ?? "";
}

function renderOutput(output) {
  if (output.output_type === "stream") {
    return toText(output.text);
  }

  if (output.output_type === "execute_result" || output.output_type === "display_data") {
    return toText(output.data?.["text/plain"]);
  }

  if (output.output_type === "error") {
    if (Array.isArray(output.traceback) && output.traceback.length > 0) {
      return output.traceback.join("\n");
    }
    return [output.ename, output.evalue].filter(Boolean).join(": ");
  }

  return "";
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

function renderNotebookHtml({ notebook, sourcePath, publicHref, modifiedAt }) {
  const language =
    notebook.metadata?.language_info?.name ?? notebook.metadata?.kernelspec?.language ?? "python";
  const kernelLabel =
    notebook.metadata?.kernelspec?.display_name ?? notebook.metadata?.language_info?.version ?? "Python";

  const cells = (notebook.cells ?? [])
    .map((cell, index) => {
      const source = toText(cell.source).trimEnd();
      const outputs = (cell.outputs ?? []).map(renderOutput).filter(Boolean);
      const prompt =
        cell.cell_type === "code"
          ? `In [${cell.execution_count ?? index + 1}]`
          : "Md";

      if (cell.cell_type === "markdown") {
        return `
          <section class="notebook-demo__cell notebook-demo__cell--markdown">
            <div class="notebook-demo__prompt" aria-hidden="true">${escapeHtml(prompt)}</div>
            <div class="notebook-demo__body">
              <div class="notebook-demo__markdown sl-markdown-content">
                ${marked.parse(escapeMarkdownHtml(source))}
              </div>
            </div>
          </section>
        `;
      }

      const renderedOutputs =
        outputs.length === 0
          ? ""
          : `
            <div class="notebook-demo__outputs">
              ${outputs
                .map(
                  (output) => `
                    <pre class="notebook-demo__output"><code>${escapeHtml(output.trim())}</code></pre>
                  `
                )
                .join("")}
            </div>
          `;

      return `
        <section class="notebook-demo__cell notebook-demo__cell--code">
          <div class="notebook-demo__prompt" aria-hidden="true">${escapeHtml(prompt)}</div>
          <div class="notebook-demo__body">
            <pre class="notebook-demo__code"><code class="language-${escapeHtml(language)}">${escapeHtml(
              source
            )}</code></pre>
            ${renderedOutputs}
          </div>
        </section>
      `;
    })
    .join("");

  return `
    <div class="notebook-demo not-content">
      <div class="notebook-demo__toolbar">
        <div class="notebook-demo__meta">
          <p class="notebook-demo__eyebrow">Interactive Notebook</p>
          <h2 class="notebook-demo__title">Notebook Source</h2>
          <p class="notebook-demo__description">
            Pre-rendered from the checked-in \`.ipynb\` file during the docs build for static delivery and search indexing.
          </p>
          <dl class="notebook-demo__details">
            <div>
              <dt>Source</dt>
              <dd><code>${escapeHtml(sourcePath)}</code></dd>
            </div>
            <div>
              <dt>Default API</dt>
              <dd><code>https://app.everruns.com/api</code></dd>
            </div>
            <div>
              <dt>Notebook File Timestamp</dt>
              <dd>${escapeHtml(modifiedAt)}</dd>
            </div>
          </dl>
        </div>
        <div class="notebook-demo__actions">
          <span class="notebook-demo__kernel">${escapeHtml(kernelLabel)}</span>
          <a class="notebook-demo__download" href="${escapeHtml(publicHref)}" download>
            Download .ipynb
          </a>
          <a
            class="notebook-demo__download"
            href="https://github.com/everruns/everruns/blob/main/${escapeHtml(sourcePath)}"
          >
            View Source
          </a>
        </div>
      </div>
      <div class="notebook-demo__cells">${cells}</div>
    </div>
  `;
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
  const modifiedAt = statSync(notebookPath).mtime.toISOString();
  const publicHref = `/notebooks/${notebookRelativeToDocs}`;

  mkdirSync(path.dirname(htmlOutputPath), { recursive: true });
  mkdirSync(path.dirname(publicNotebookPath), { recursive: true });
  copyFileSync(notebookPath, publicNotebookPath);

  const renderedHtml = renderNotebookHtml({
    notebook,
    sourcePath: notebookKey,
    publicHref,
    modifiedAt,
  });

  writeFileSync(htmlOutputPath, renderedHtml, "utf8");

  manifest[notebookKey] = {
    htmlFile: toPosix(htmlRelativePath),
    publicHref,
    route: deriveRoute(docPath, data),
    docPage: docRelativePath,
    notebookFile: notebookKey,
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
