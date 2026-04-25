"use client";

/**
 * File Preview Components
 *
 * Uses Streamdown with @streamdown/code for Shiki-based syntax highlighting,
 * matching the styling used in chat messages.
 *
 * SVG TRUST BOUNDARY (see also `specs/threat-model.md` TM-WEB-009):
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
  | "text"
  | "binary";

export function getPreviewType(extension: string, encoding: "text" | "base64"): PreviewType {
  const ext = extension.toLowerCase();

  // SVG previews go through the sandboxed iframe path regardless of how the
  // bytes are delivered; see SVG TRUST BOUNDARY at the top of this module.
  if (ext === "svg") return "svg";

  if (encoding === "base64") {
    if (IMAGE_EXTENSIONS.has(ext)) return "image";
    return "binary";
  }

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
        <Streamdown plugins={{ code }}>{body}</Streamdown>
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

export function FilePreview({ content, extension, encoding }: PreviewProps) {
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
    case "image":
      return (
        <ImagePreview content={content} extension={extension} fileName={`file.${extension}`} />
      );
    default:
      return null;
  }
}
