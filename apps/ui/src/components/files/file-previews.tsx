"use client";

/**
 * File Preview Components
 *
 * Uses Streamdown with @streamdown/code for Shiki-based syntax highlighting,
 * matching the styling used in chat messages.
 *
 * SVG TRUST BOUNDARY (see also `knowledge/security/threat-model.md` TM-WEB-009):
 * SVG files are user-supplied bytes. Rendering attacker-controlled SVG via
 * `<img src="data:image/svg+xml;base64,…">` was blocked in PR #1513 because
 * `<img>`-loaded SVG can carry `<script>` and `on*` event-handler XSS in
 * some browsers. We re-enable preview by rendering inside an
 * `<iframe sandbox="" srcDoc=...>`:
 *
 *   1. `sandbox=""` denies all sandbox flags (scripts, forms, top-nav,
 *      same-origin, popups), so even script-bearing SVG cannot run.
 *   2. The srcDoc carries a strict CSP meta tag
 *      (`default-src 'none'; style-src 'unsafe-inline'; img-src data:`) as
 *      defense in depth: external fetches, scripts, and cross-origin loads
 *      are blocked even if a sandbox flag were ever loosened.
 *   3. SVG bytes go into the iframe body; `<script>`, `on*`, `javascript:`,
 *      and `<foreignObject>` HTML are still parsed by the iframe DOM but
 *      cannot execute or fetch under either gate.
 *
 * The trust gate is enforced in `SVGPreview` below; `getPreviewType` routes
 * `.svg` files to that component instead of `<img>`-based `ImagePreview`.
 *
 * HTML TRUST BOUNDARY (see also `knowledge/security/threat-model.md` TM-WEB-010):
 * `.html`/`.htm` files are user-supplied documents that may legitimately need
 * to run JavaScript to render. In every mode the document runs in an opaque
 * origin (sandbox WITHOUT `allow-same-origin`), so it cannot read everruns
 * cookies, localStorage, or the parent DOM — verified: `document.cookie`,
 * `parent.document`, and `localStorage` all throw `SecurityError` and
 * `window.origin` is `null`. `allow-top-navigation`/`allow-forms`/
 * `allow-popups`/`allow-modals` are always omitted, so no redirects, form
 * posts, popups, or dialogs. (Never add `allow-same-origin` alongside
 * `allow-scripts` for untrusted content: the script could then reach out and
 * strip its own `sandbox`.)
 *
 * There are two render modes because of a CSP-inheritance subtlety:
 *
 *   1. Server-backed (`src`, preferred — used by the file viewer): the iframe
 *      loads the sandboxed preview endpoint
 *      (`/v1/workspaces/{id}/fs/_/preview/{path}`), whose RESPONSE carries
 *      `Content-Security-Policy: sandbox allow-scripts; …`. A network response
 *      does NOT inherit the app's strict `script-src 'self'`, so the page's
 *      JavaScript actually executes (verified under the real app CSP).
 *   2. Static fallback (`srcDoc`, e.g. initial-files preview where content is
 *      not yet on the server): an `about:srcdoc` document INHERITS the app's
 *      `script-src 'self'`, which a child CSP cannot loosen, so inline scripts
 *      do NOT run — CSS/markup still render. We also inject a hardening CSP
 *      `<meta>` (`object-src`/`base-uri`/`form-action 'none'`) as defense in
 *      depth. This mode is safe but non-interactive.
 *
 * PDF TRUST BOUNDARY (see also `knowledge/security/threat-model.md` TM-WEB-011):
 * PDFs render via `<iframe src="data:application/pdf;base64,…">`. Chromium
 * disables its built-in PDF viewer inside *any* sandboxed iframe (verified:
 * `sandbox=""`, `allow-scripts`, and `allow-scripts allow-same-origin` all
 * fail to render, for both `data:` and `blob:` sources), so `sandbox` is not
 * available here. Security instead comes from:
 *
 *   1. The `data:` URL gives the frame an opaque origin, so it cannot read
 *      everruns cookies/DOM even without `sandbox`.
 *   2. The PDF document runs inside Chromium's out-of-process PDF viewer, an
 *      isolated context that cannot script the embedding page.
 *   3. Forcing `application/pdf` means a mislabeled `.pdf` (e.g. HTML bytes)
 *      is parsed as a (broken) PDF, never executed as HTML.
 *
 * The parent CSP needs `frame-src 'self' data:` for the PDF `data:` frame to
 * load; see `crates/server/src/app_builder.rs`.
 */

import { useMemo } from "react";
import { Streamdown } from "streamdown";
import { code } from "@streamdown/code";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { AlertCircle } from "lucide-react";
import { MarkdownLink } from "@/components/markdown/markdown-link";
import "./file-previews.css";

// File extension categorization
const CODE_EXTENSIONS = new Set([
  "js",
  "jsx",
  "ts",
  "tsx",
  "py",
  "rs",
  "go",
  "java",
  "c",
  "cpp",
  "h",
  "hpp",
  "rb",
  "php",
  "sql",
  "sh",
  "bash",
  "zsh",
  "yml",
  "yaml",
  "toml",
  "xml",
  "html",
  "css",
  "scss",
  "sass",
  "less",
  "vue",
  "svelte",
]);

const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp", "bmp", "ico"]);
const markdownComponents = { a: MarkdownLink };

// Map file extensions to Shiki language identifiers
const EXTENSION_TO_LANG: Record<string, string> = {
  js: "javascript",
  jsx: "jsx",
  ts: "typescript",
  tsx: "tsx",
  py: "python",
  rs: "rust",
  go: "go",
  java: "java",
  c: "c",
  cpp: "cpp",
  h: "c",
  hpp: "cpp",
  rb: "ruby",
  php: "php",
  sql: "sql",
  sh: "bash",
  bash: "bash",
  zsh: "bash",
  yml: "yaml",
  yaml: "yaml",
  toml: "toml",
  xml: "xml",
  html: "html",
  css: "css",
  scss: "scss",
  sass: "sass",
  less: "less",
  vue: "vue",
  svelte: "svelte",
  json: "json",
};

export type PreviewType =
  | "code"
  | "csv"
  | "json"
  | "markdown"
  | "image"
  | "svg"
  | "html"
  | "pdf"
  | "text"
  | "binary";

export function getPreviewType(extension: string, encoding: "text" | "base64"): PreviewType {
  const ext = extension.toLowerCase();

  // SVG previews go through the sandboxed iframe path regardless of how the
  // bytes are delivered; see SVG TRUST BOUNDARY at the top of this module.
  if (ext === "svg") return "svg";

  if (encoding === "base64") {
    // PDFs render in the built-in viewer via a data: URL; see PDF TRUST
    // BOUNDARY at the top of this module.
    if (ext === "pdf") return "pdf";
    if (IMAGE_EXTENSIONS.has(ext)) return "image";
    return "binary";
  }

  // HTML files render (with JS) inside a sandboxed, opaque-origin iframe;
  // see HTML TRUST BOUNDARY at the top of this module. Routed before the
  // generic code path so the default "View" is the rendered page.
  if (ext === "html" || ext === "htm") return "html";
  if (ext === "md" || ext === "markdown") return "markdown";
  if (ext === "json") return "json";
  if (ext === "csv") return "csv";
  if (CODE_EXTENSIONS.has(ext)) return "code";
  return "text";
}

export function canPreview(extension: string, encoding: "text" | "base64"): boolean {
  const type = getPreviewType(extension, encoding);
  return type !== "binary" && type !== "text";
}

interface PreviewProps {
  content: string;
  extension: string;
  encoding: "text" | "base64";
  /**
   * Browser-navigable URL to the sandboxed HTML preview endpoint. When provided
   * for an HTML file, the preview loads it via `<iframe src>` so JavaScript runs
   * (see HtmlPreview). Omitted for surfaces whose content is not yet on the
   * server (e.g. initial-files preview), which fall back to a static render.
   */
  htmlPreviewSrc?: string;
}

export function CodePreview({ content, extension }: { content: string; extension: string }) {
  const lang = EXTENSION_TO_LANG[extension.toLowerCase()] || "text";
  const markdown = useMemo(() => `\`\`\`${lang}\n${content}\n\`\`\``, [content, lang]);

  return (
    <ScrollArea className="h-full">
      <div className="file-preview-streamdown text-sm">
        <Streamdown plugins={{ code }}>{markdown}</Streamdown>
      </div>
    </ScrollArea>
  );
}

function parseCSV(content: string): { headers: string[]; rows: string[][] } {
  const lines = content.split("\n").filter((line) => line.trim());
  if (lines.length === 0) return { headers: [], rows: [] };

  // Simple CSV parser (handles quoted fields)
  const parseLine = (line: string): string[] => {
    const result: string[] = [];
    let current = "";
    let inQuotes = false;

    for (let i = 0; i < line.length; i++) {
      const char = line[i];
      if (char === '"') {
        if (inQuotes && line[i + 1] === '"') {
          current += '"';
          i++;
        } else {
          inQuotes = !inQuotes;
        }
      } else if (char === "," && !inQuotes) {
        result.push(current.trim());
        current = "";
      } else {
        current += char;
      }
    }
    result.push(current.trim());
    return result;
  };

  const headers = parseLine(lines[0]);
  const rows = lines.slice(1).map(parseLine);

  return { headers, rows };
}

export function CSVPreview({ content }: { content: string }) {
  const { headers, rows } = useMemo(() => parseCSV(content), [content]);

  if (headers.length === 0) {
    return (
      <div className="p-4 text-sm text-muted-foreground text-center">
        <AlertCircle className="h-8 w-8 mx-auto mb-2 text-gray-300" />
        <p>Empty or invalid CSV</p>
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-4">
        <Table>
          <TableHeader>
            <TableRow>
              {headers.map((header, i) => (
                <TableHead key={i} className="bg-muted font-semibold">
                  {header}
                </TableHead>
              ))}
            </TableRow>
          </TableHeader>
          <TableBody>
            {rows.map((row, i) => (
              <TableRow key={i}>
                {row.map((cell, j) => (
                  <TableCell key={j}>{cell}</TableCell>
                ))}
              </TableRow>
            ))}
          </TableBody>
        </Table>
        <div className="mt-2 text-xs text-muted-foreground">
          {rows.length} rows, {headers.length} columns
        </div>
      </div>
    </ScrollArea>
  );
}

export function JSONPreview({ content }: { content: string }) {
  const { formatted, markdown } = useMemo(() => {
    try {
      const parsed = JSON.parse(content);
      const formattedJson = JSON.stringify(parsed, null, 2);
      return {
        formatted: formattedJson,
        markdown: `\`\`\`json\n${formattedJson}\n\`\`\``,
      };
    } catch {
      return { formatted: null, markdown: null };
    }
  }, [content]);

  if (!formatted || !markdown) {
    return (
      <div className="p-4 text-sm text-muted-foreground text-center">
        <AlertCircle className="h-8 w-8 mx-auto mb-2 text-yellow-500" />
        <p>Invalid JSON</p>
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="file-preview-streamdown text-sm">
        <Streamdown plugins={{ code }}>{markdown}</Streamdown>
      </div>
    </ScrollArea>
  );
}

/**
 * Parse YAML frontmatter from markdown content.
 * Returns the frontmatter entries (key-value pairs) and the remaining body.
 * Only detects frontmatter that starts at the very beginning of the content.
 */
export function parseFrontmatter(content: string): {
  entries: { key: string; value: string }[];
  body: string;
} {
  // Frontmatter must start at the very beginning with ---
  if (!content.startsWith("---")) {
    return { entries: [], body: content };
  }

  // Find the closing ---
  const endIndex = content.indexOf("\n---", 3);
  if (endIndex === -1) {
    return { entries: [], body: content };
  }

  const frontmatterBlock = content.slice(4, endIndex).trim();
  const body = content.slice(endIndex + 4).trimStart();

  if (!frontmatterBlock) {
    return { entries: [], body };
  }

  // Parse simple YAML key: value pairs
  // Handles multiline values by treating indented continuation lines as part of the previous value
  const entries: { key: string; value: string }[] = [];
  const lines = frontmatterBlock.split("\n");
  let currentKey = "";
  let currentValue = "";

  for (const line of lines) {
    // Check if this is a new key-value pair (not indented, has colon)
    const match = line.match(/^([a-zA-Z0-9_-]+)\s*:\s*(.*)/);
    if (match) {
      // Save previous entry
      if (currentKey) {
        entries.push({ key: currentKey, value: currentValue.trim() });
      }
      currentKey = match[1];
      currentValue = match[2];
    } else if (currentKey && (line.startsWith("  ") || line.startsWith("\t"))) {
      // Continuation line for multiline value
      currentValue += "\n" + line.trimStart();
    }
  }

  // Save last entry
  if (currentKey) {
    entries.push({ key: currentKey, value: currentValue.trim() });
  }

  return { entries, body };
}

function FrontmatterBlock({ entries }: { entries: { key: string; value: string }[] }) {
  return (
    <div className="file-preview-frontmatter">
      <table>
        <tbody>
          {entries.map(({ key, value }) => (
            <tr key={key}>
              <td className="file-preview-frontmatter-key">{key}</td>
              <td className="file-preview-frontmatter-value">
                {value || <span className="empty">—</span>}
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

export function MarkdownPreview({ content }: { content: string }) {
  const { entries, body } = useMemo(() => parseFrontmatter(content), [content]);

  return (
    <ScrollArea className="h-full">
      <div className="file-preview-streamdown text-sm p-4">
        {entries.length > 0 && <FrontmatterBlock entries={entries} />}
        <Streamdown plugins={{ code }} components={markdownComponents}>
          {body}
        </Streamdown>
      </div>
    </ScrollArea>
  );
}

export function ImagePreview({
  content,
  extension,
  fileName,
}: {
  content: string;
  extension: string;
  fileName: string;
}) {
  const mimeType = useMemo(() => {
    const mimeTypes: Record<string, string> = {
      png: "image/png",
      jpg: "image/jpeg",
      jpeg: "image/jpeg",
      gif: "image/gif",
      webp: "image/webp",
      bmp: "image/bmp",
      ico: "image/x-icon",
    };
    return mimeTypes[extension.toLowerCase()] || "image/png";
  }, [extension]);

  const dataUrl = `data:${mimeType};base64,${content}`;

  return (
    <ScrollArea className="h-full">
      <div className="p-4 flex items-center justify-center min-h-full">
        {/* eslint-disable-next-line @next/next/no-img-element -- preview source is a data: URL from user-uploaded bytes; next/image's optimizer/loaders do not handle data: URLs */}
        <img
          src={dataUrl}
          alt={fileName}
          className="max-w-full max-h-[calc(100vh-200px)] object-contain"
        />
      </div>
    </ScrollArea>
  );
}

/**
 * Decode user-supplied SVG bytes to a string.
 *
 * `text` encoding is already an SVG source string. `base64` encoding decodes
 * via `atob` and reconstructs UTF-8 (SVG can carry non-ASCII glyph names,
 * gradient stop labels, foreign-object text, etc.). Decoding only converts
 * bytes to characters — it does NOT execute or sanitize anything; the
 * sandboxed iframe + CSP do that work in `SVGPreview`.
 */
function decodeSvgSource(content: string, encoding: "text" | "base64"): string {
  if (encoding !== "base64") {
    return content;
  }
  // Strip whitespace before `atob`. PEM-style and HTTP-multipart base64 carry
  // line breaks; many APIs and pasted-from-clipboard payloads include
  // newlines that strict `atob` rejects.
  const normalized = content.replace(/\s+/g, "");
  try {
    const binary = atob(normalized);
    if (typeof TextDecoder !== "undefined") {
      const bytes = new Uint8Array(binary.length);
      for (let i = 0; i < binary.length; i++) bytes[i] = binary.charCodeAt(i);
      return new TextDecoder("utf-8", { fatal: false }).decode(bytes);
    }
    return binary;
  } catch {
    return "";
  }
}

export function SVGPreview({
  content,
  encoding,
}: {
  content: string;
  encoding: "text" | "base64";
}) {
  const svgSource = useMemo(() => decodeSvgSource(content, encoding), [content, encoding]);

  // Strict CSP for the iframe document. `default-src 'none'` blocks all
  // network I/O the SVG might attempt (xlink:href to external URLs, image
  // tags, etc.). `style-src 'unsafe-inline'` is required so SVG `<style>`
  // and inline `style="..."` attributes — which are the legitimate way to
  // color/size SVGs — still apply. `img-src data:` allows base64 raster
  // tiles inlined inside the SVG; remote images stay blocked.
  const srcDoc = useMemo(() => {
    const csp = "default-src 'none'; style-src 'unsafe-inline'; img-src data:";
    // SVG body is inserted verbatim; the iframe's `sandbox=""` + CSP gate
    // any executable content. No string-level guard around `</body>` — even
    // if an SVG contains that substring, the iframe's HTML parser handles a
    // premature body close gracefully (subsequent content reopens implicitly)
    // and the trust gate (sandbox + CSP) is unaffected.
    return `<!DOCTYPE html>
<html><head>
<meta charset="utf-8">
<meta http-equiv="Content-Security-Policy" content="${csp}">
<style>html,body{margin:0;padding:0;height:100%;display:flex;align-items:center;justify-content:center;background:transparent}svg{max-width:100%;max-height:100%}</style>
</head>
<body>${svgSource}</body></html>`;
  }, [svgSource]);

  if (!svgSource.trim()) {
    return (
      <div className="p-4 text-sm text-muted-foreground text-center">
        <AlertCircle className="h-8 w-8 mx-auto mb-2 text-yellow-500" />
        <p>Empty or invalid SVG</p>
      </div>
    );
  }

  return (
    <ScrollArea className="h-full">
      <div className="p-4 flex items-center justify-center min-h-full">
        <iframe
          title="SVG preview"
          // Empty `sandbox` means deny every flag — no scripts, no forms, no
          // top-nav, no same-origin, no popups. Combined with the CSP meta
          // tag inside `srcDoc`, this is the strongest browser-side gate
          // available for inline SVG. See SVG TRUST BOUNDARY at module top.
          sandbox=""
          srcDoc={srcDoc}
          className="w-full max-w-full max-h-[calc(100vh-200px)] aspect-square border-0 bg-white dark:bg-neutral-950 rounded"
        />
      </div>
    </ScrollArea>
  );
}

// Defense-in-depth CSP for the HTML preview iframe. It deliberately does NOT
// constrain `script-src`/`default-src` — the preview must run JavaScript — but
// it blocks plugin embeds (`object-src 'none'`), `<base>` hijacking of relative
// URLs (`base-uri 'none'`), and form submissions (`form-action 'none'`). The
// real credential-theft gate is the iframe's opaque origin (no
// `allow-same-origin`); this CSP is belt-and-suspenders. See HTML TRUST
// BOUNDARY at the top of this module.
const HTML_PREVIEW_CSP = "object-src 'none'; base-uri 'none'; form-action 'none'";

/**
 * Inject the hardening CSP meta tag as the FIRST child of <head> so it governs
 * everything parsed after it. Falls back to wrapping in <head>/prepending when
 * the user's HTML omits those tags (browsers still honor a leading meta CSP).
 * Inserting at the head's start — before any user script — is what makes the
 * policy apply to those scripts.
 */
function injectHtmlPreviewCsp(html: string): string {
  const meta = `<meta http-equiv="Content-Security-Policy" content="${HTML_PREVIEW_CSP}">`;
  const headOpen = /<head[^>]*>/i;
  if (headOpen.test(html)) {
    return html.replace(headOpen, (match) => `${match}${meta}`);
  }
  const htmlOpen = /<html[^>]*>/i;
  if (htmlOpen.test(html)) {
    return html.replace(htmlOpen, (match) => `${match}<head>${meta}</head>`);
  }
  return `${meta}${html}`;
}

export function HtmlPreview({ content, src }: { content: string; src?: string }) {
  const srcDoc = useMemo(() => injectHtmlPreviewCsp(content), [content]);

  // Server-backed mode (preferred): the iframe loads the sandboxed preview
  // endpoint, whose response carries `Content-Security-Policy: sandbox
  // allow-scripts`. Because a network response does NOT inherit the app's
  // strict `script-src 'self'` (a `srcdoc` document would), the page's
  // JavaScript actually runs — while the opaque sandbox still blocks cookie/DOM
  // access and top-frame navigation. The iframe `sandbox` attribute is
  // defense-in-depth on top of the server CSP.
  if (src) {
    return (
      <iframe
        title="HTML preview"
        sandbox="allow-scripts"
        referrerPolicy="no-referrer"
        src={src}
        className="w-full h-full border-0 bg-white"
      />
    );
  }

  // Static fallback (e.g. initial-files preview, where the content is not yet on
  // the server): rendered from `srcDoc` in an opaque-origin sandbox. Fully
  // isolated, but inline scripts do NOT run because the `about:srcdoc` document
  // inherits the app's `script-src 'self'`. CSS/markup still render. See HTML
  // TRUST BOUNDARY at the top of this module.
  return (
    <iframe
      title="HTML preview"
      sandbox="allow-scripts"
      referrerPolicy="no-referrer"
      srcDoc={srcDoc}
      className="w-full h-full border-0 bg-white"
    />
  );
}

export function PdfPreview({ content }: { content: string }) {
  // Chromium disables its PDF viewer inside any sandboxed iframe, so we cannot
  // use `sandbox` here. The data: URL gives the frame an opaque origin (no
  // cookie/DOM access) and forcing `application/pdf` prevents HTML execution
  // for mislabeled files. See PDF TRUST BOUNDARY at the top of this module.
  const dataUrl = useMemo(
    () => `data:application/pdf;base64,${content.replace(/\s+/g, "")}`,
    [content],
  );

  if (!content.trim()) {
    return (
      <div className="p-4 text-sm text-muted-foreground text-center">
        <AlertCircle className="h-8 w-8 mx-auto mb-2 text-yellow-500" />
        <p>Empty or invalid PDF</p>
      </div>
    );
  }

  return (
    <iframe title="PDF preview" src={dataUrl} className="w-full h-full border-0 bg-neutral-50" />
  );
}

export function FilePreview({ content, extension, encoding, htmlPreviewSrc }: PreviewProps) {
  const previewType = getPreviewType(extension, encoding);

  switch (previewType) {
    case "code":
      return <CodePreview content={content} extension={extension} />;
    case "csv":
      return <CSVPreview content={content} />;
    case "json":
      return <JSONPreview content={content} />;
    case "markdown":
      return <MarkdownPreview content={content} />;
    case "svg":
      return <SVGPreview content={content} encoding={encoding} />;
    case "html":
      return <HtmlPreview content={content} src={htmlPreviewSrc} />;
    case "pdf":
      return <PdfPreview content={content} />;
    case "image":
      return (
        <ImagePreview content={content} extension={extension} fileName={`file.${extension}`} />
      );
    default:
      return null;
  }
}
