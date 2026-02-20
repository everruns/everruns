"use client";

/**
 * File Preview Components
 *
 * Uses Streamdown with @streamdown/code for Shiki-based syntax highlighting,
 * matching the styling used in chat messages.
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

// Streamdown styling for code previews
const previewStyles = `
  .file-preview-streamdown pre { background: var(--color-muted); padding: 1em; overflow-x: auto; margin: 0; }
  .file-preview-streamdown pre code { background: none; padding: 0; }
  .file-preview-streamdown [data-streamdown-code] { border-radius: 0; }
  .file-preview-streamdown p { margin: 0.5em 0; }
  .file-preview-streamdown ul, .file-preview-streamdown ol { margin: 0.5em 0; padding-left: 1.5em; }
  .file-preview-streamdown li { margin: 0.25em 0; }
  .file-preview-streamdown code:not(pre code) { background: var(--color-muted); padding: 0.2em 0.4em; font-size: 0.9em; }
  .file-preview-streamdown blockquote { border-left: 3px solid var(--color-border); padding-left: 1em; margin: 0.5em 0; color: var(--color-muted-foreground); }
  .file-preview-streamdown hr { border: none; border-top: 1px solid var(--color-border); margin: 1em 0; }
  .file-preview-streamdown a { color: var(--color-primary); text-decoration: underline; }
  .file-preview-streamdown strong { font-weight: 600; }
  .file-preview-streamdown em { font-style: italic; }
  .file-preview-streamdown table { border-collapse: collapse; width: 100%; margin: 0.5em 0; }
  .file-preview-streamdown th, .file-preview-streamdown td { border: 1px solid var(--color-border); padding: 0.5em; text-align: left; }
  .file-preview-streamdown th { background: var(--color-muted); font-weight: 600; }
  .file-preview-streamdown img { max-width: 100%; height: auto; }
  .file-preview-streamdown h1 { font-size: 1.5em; font-weight: 700; margin: 1em 0 0.5em; }
  .file-preview-streamdown h2 { font-size: 1.25em; font-weight: 600; margin: 1em 0 0.5em; }
  .file-preview-streamdown h3 { font-size: 1.1em; font-weight: 600; margin: 1em 0 0.5em; }

  /* Frontmatter metadata block */
  .file-preview-frontmatter { background: var(--color-muted); border: 1px solid var(--color-border); border-radius: 6px; padding: 0.75em 1em; margin-bottom: 1.5em; }
  .file-preview-frontmatter table { width: 100%; border-collapse: collapse; }
  .file-preview-frontmatter tr:not(:last-child) td { padding-bottom: 0.25em; }
  .file-preview-frontmatter-key { color: var(--color-muted-foreground); font-weight: 500; white-space: nowrap; padding-right: 1em; vertical-align: top; width: 1%; font-size: 0.85em; }
  .file-preview-frontmatter-value { color: var(--color-foreground); word-break: break-word; font-size: 0.85em; }
  .file-preview-frontmatter-value .empty { color: var(--color-muted-foreground); }

  /* GitHub-style alerts */
  .file-preview-streamdown .markdown-alert { padding: 0.5em 1em; margin: 0.5em 0; border-left: 4px solid; }
  .file-preview-streamdown .markdown-alert-title { display: flex; align-items: center; gap: 0.5em; font-weight: 600; margin-bottom: 0.25em; }
  .file-preview-streamdown .markdown-alert-title svg { width: 1em; height: 1em; }
  .file-preview-streamdown .markdown-alert-note { border-color: #0969da; background: rgba(9, 105, 218, 0.1); }
  .file-preview-streamdown .markdown-alert-note .markdown-alert-title { color: #0969da; }
  .file-preview-streamdown .markdown-alert-tip { border-color: #1a7f37; background: rgba(26, 127, 55, 0.1); }
  .file-preview-streamdown .markdown-alert-tip .markdown-alert-title { color: #1a7f37; }
  .file-preview-streamdown .markdown-alert-important { border-color: #8250df; background: rgba(130, 80, 223, 0.1); }
  .file-preview-streamdown .markdown-alert-important .markdown-alert-title { color: #8250df; }
  .file-preview-streamdown .markdown-alert-warning { border-color: #9a6700; background: rgba(154, 103, 0, 0.1); }
  .file-preview-streamdown .markdown-alert-warning .markdown-alert-title { color: #9a6700; }
  .file-preview-streamdown .markdown-alert-caution { border-color: #cf222e; background: rgba(207, 34, 46, 0.1); }
  .file-preview-streamdown .markdown-alert-caution .markdown-alert-title { color: #cf222e; }
`;

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

const IMAGE_EXTENSIONS = new Set(["png", "jpg", "jpeg", "gif", "webp", "svg", "bmp", "ico"]);

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

export type PreviewType = "code" | "csv" | "json" | "markdown" | "image" | "text" | "binary";

export function getPreviewType(extension: string, encoding: "text" | "base64"): PreviewType {
  const ext = extension.toLowerCase();

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
      <style>{previewStyles}</style>
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
      <style>{previewStyles}</style>
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
      <style>{previewStyles}</style>
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
      svg: "image/svg+xml",
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
    case "image":
      return (
        <ImagePreview content={content} extension={extension} fileName={`file.${extension}`} />
      );
    default:
      return null;
  }
}
