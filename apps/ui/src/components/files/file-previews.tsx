"use client";

import { useMemo } from "react";
import { ScrollArea } from "@/components/ui/scroll-area";
import {
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
} from "@/components/ui/table";
import { Markdown } from "@/components/ui/markdown";
import { AlertCircle } from "lucide-react";

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

// Simple syntax highlighting using CSS classes
function highlightCode(code: string, extension: string): React.ReactNode {
  // Basic tokenization for common patterns
  const lines = code.split("\n");

  return lines.map((line, i) => (
    <div key={i} className="code-line flex">
      <span className="code-line-number select-none text-muted-foreground/50 pr-4 text-right w-12 shrink-0">
        {i + 1}
      </span>
      <span className="code-line-content flex-1">
        <HighlightedLine line={line} extension={extension} />
      </span>
    </div>
  ));
}

function HighlightedLine({ line, extension }: { line: string; extension: string }) {
  // Very simple syntax highlighting
  const tokens = tokenizeLine(line, extension);
  return (
    <>
      {tokens.map((token, i) => (
        <span key={i} className={token.className}>
          {token.text}
        </span>
      ))}
    </>
  );
}

interface Token {
  text: string;
  className: string;
}

function tokenizeLine(line: string, extension: string): Token[] {
  const tokens: Token[] = [];

  // Comment detection
  const commentPatterns: Record<string, RegExp> = {
    py: /^(\s*)(#.*)/,
    sh: /^(\s*)(#.*)/,
    bash: /^(\s*)(#.*)/,
    rb: /^(\s*)(#.*)/,
    yml: /^(\s*)(#.*)/,
    yaml: /^(\s*)(#.*)/,
    toml: /^(\s*)(#.*)/,
    js: /^(\s*)(\/\/.*)/,
    jsx: /^(\s*)(\/\/.*)/,
    ts: /^(\s*)(\/\/.*)/,
    tsx: /^(\s*)(\/\/.*)/,
    java: /^(\s*)(\/\/.*)/,
    go: /^(\s*)(\/\/.*)/,
    rs: /^(\s*)(\/\/.*)/,
    c: /^(\s*)(\/\/.*)/,
    cpp: /^(\s*)(\/\/.*)/,
    css: /^(\s*)(\/\*.*\*\/)/,
    html: /^(\s*)(<!--.*-->)/,
    xml: /^(\s*)(<!--.*-->)/,
  };

  const commentPattern = commentPatterns[extension];
  if (commentPattern) {
    const match = line.match(commentPattern);
    if (match) {
      if (match[1]) tokens.push({ text: match[1], className: "" });
      tokens.push({ text: match[2], className: "text-muted-foreground italic" });
      return tokens;
    }
  }

  // Keywords by language
  const jsKeywords =
    /\b(const|let|var|function|return|if|else|for|while|class|import|export|from|async|await|try|catch|throw|new|this|typeof|instanceof)\b/g;
  const pyKeywords =
    /\b(def|class|return|if|elif|else|for|while|import|from|as|try|except|raise|with|lambda|yield|async|await|True|False|None)\b/g;
  const rsKeywords =
    /\b(fn|let|mut|const|struct|enum|impl|trait|pub|use|mod|match|if|else|for|while|loop|return|self|Self|async|await|move|where)\b/g;
  const goKeywords =
    /\b(func|var|const|type|struct|interface|package|import|if|else|for|range|return|defer|go|chan|select|case|default|switch|break|continue)\b/g;

  const keywordPatterns: Record<string, RegExp> = {
    js: jsKeywords,
    jsx: jsKeywords,
    ts: jsKeywords,
    tsx: jsKeywords,
    py: pyKeywords,
    rs: rsKeywords,
    go: goKeywords,
  };

  // String detection
  const stringPattern = /(["'`])(?:(?!\1)[^\\]|\\.)*\1/g;

  // Number detection
  const numberPattern = /\b\d+\.?\d*\b/g;

  // Build highlighted line
  let remaining = line;
  let lastIndex = 0;
  const keywordPattern = keywordPatterns[extension];

  // For simplicity, just highlight strings, numbers, and keywords
  const allPatterns: { pattern: RegExp; className: string }[] = [
    { pattern: stringPattern, className: "text-green-600 dark:text-green-400" },
    { pattern: numberPattern, className: "text-blue-600 dark:text-blue-400" },
  ];

  if (keywordPattern) {
    allPatterns.push({ pattern: keywordPattern, className: "text-purple-600 dark:text-purple-400" });
  }

  // Simple approach: just return the line with basic highlighting
  let result = line;
  for (const { pattern, className } of allPatterns) {
    result = result.replace(pattern, (match) => `\x00${className}\x01${match}\x02`);
  }

  // Parse the result back into tokens
  const parts = result.split(/(\x00[^\x01]+\x01[^\x02]*\x02)/);
  for (const part of parts) {
    const match = part.match(/\x00([^\x01]+)\x01([^\x02]*)\x02/);
    if (match) {
      tokens.push({ text: match[2], className: match[1] });
    } else if (part) {
      tokens.push({ text: part, className: "" });
    }
  }

  return tokens.length > 0 ? tokens : [{ text: line, className: "" }];
}

export function CodePreview({ content, extension }: { content: string; extension: string }) {
  const highlighted = useMemo(() => highlightCode(content, extension), [content, extension]);

  return (
    <ScrollArea className="h-full">
      <pre className="p-4 text-sm font-mono">{highlighted}</pre>
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
  const formatted = useMemo(() => {
    try {
      const parsed = JSON.parse(content);
      return JSON.stringify(parsed, null, 2);
    } catch {
      return null;
    }
  }, [content]);

  if (!formatted) {
    return (
      <div className="p-4 text-sm text-muted-foreground text-center">
        <AlertCircle className="h-8 w-8 mx-auto mb-2 text-yellow-500" />
        <p>Invalid JSON</p>
      </div>
    );
  }

  // Highlight JSON
  const highlighted = useMemo(() => {
    const lines = formatted.split("\n");
    return lines.map((line, i) => {
      // Highlight keys
      let result = line.replace(
        /"([^"]+)":/g,
        '<span class="text-purple-600 dark:text-purple-400">"$1"</span>:',
      );
      // Highlight string values
      result = result.replace(
        /: "([^"]*)"/g,
        ': <span class="text-green-600 dark:text-green-400">"$1"</span>',
      );
      // Highlight numbers
      result = result.replace(
        /: (\d+\.?\d*)/g,
        ': <span class="text-blue-600 dark:text-blue-400">$1</span>',
      );
      // Highlight booleans and null
      result = result.replace(
        /: (true|false|null)/g,
        ': <span class="text-orange-600 dark:text-orange-400">$1</span>',
      );

      return (
        <div key={i} className="code-line flex">
          <span className="code-line-number select-none text-muted-foreground/50 pr-4 text-right w-12 shrink-0">
            {i + 1}
          </span>
          <span
            className="code-line-content flex-1"
            dangerouslySetInnerHTML={{ __html: result }}
          />
        </div>
      );
    });
  }, [formatted]);

  return (
    <ScrollArea className="h-full">
      <pre className="p-4 text-sm font-mono">{highlighted}</pre>
    </ScrollArea>
  );
}

export function MarkdownPreview({ content }: { content: string }) {
  return (
    <ScrollArea className="h-full">
      <div className="p-4">
        <Markdown content={content} variant="compact" />
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
      return <ImagePreview content={content} extension={extension} fileName={`file.${extension}`} />;
    default:
      return null;
  }
}
