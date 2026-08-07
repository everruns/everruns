"use client";

import { useEffect, useState, type CSSProperties, type ReactNode } from "react";
import type { HighlightResult } from "@streamdown/code";
import { Check, Copy } from "lucide-react";
import { Button } from "./button";
import { cn } from "@/lib/utils";

type CodeHighlighter = (typeof import("@streamdown/code"))["code"];

let highlighterPromise: Promise<CodeHighlighter> | undefined;

function loadHighlighter() {
  highlighterPromise ??= import("@streamdown/code").then(({ code }) => code);
  return highlighterPromise;
}

export interface CodeBlockSample {
  /** Tab label shown when there is more than one sample. */
  label: string;
  /** Shiki language identifier used for syntax highlighting. */
  language?: string;
  code: string;
}

interface CodeBlockProps {
  samples: CodeBlockSample[];
  className?: string;
}

/**
 * Monospaced code block with optional language tabs, syntax highlighting, and
 * a copy button. Highlighting is lazy-loaded so code samples do not add Shiki
 * to the initial route bundle.
 */
export function CodeBlock({ samples, className }: CodeBlockProps) {
  const [active, setActive] = useState(0);
  const [copied, setCopied] = useState(false);
  const current = samples[active] ?? samples[0];

  if (!current) return null;

  const handleCopy = async () => {
    try {
      await navigator.clipboard.writeText(current.code);
      setCopied(true);
      setTimeout(() => setCopied(false), 1500);
    } catch {
      // Clipboard can reject in insecure contexts or when permission is denied;
      // swallow so the click never surfaces an unhandled rejection.
    }
  };

  return (
    <div className={cn("overflow-hidden rounded-md border bg-muted", className)}>
      <div className="flex items-center justify-between gap-2 border-b bg-muted/50 px-2 py-1">
        <div className="flex flex-wrap gap-0.5" role="group" aria-label="Code language">
          {samples.length > 1 &&
            samples.map((sample, index) => (
              <button
                key={`${index}-${sample.label}`}
                type="button"
                aria-pressed={index === active}
                onClick={() => {
                  setActive(index);
                  setCopied(false);
                }}
                className={cn(
                  "rounded-sm px-2.5 py-1 text-xs font-medium transition-colors focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring",
                  index === active
                    ? "bg-background text-foreground shadow-sm ring-1 ring-border"
                    : "text-muted-foreground hover:bg-background/60 hover:text-foreground",
                )}
              >
                {sample.label}
              </button>
            ))}
          {samples.length === 1 && (
            <span className="px-2 py-1 text-xs font-medium text-muted-foreground">
              {current.label}
            </span>
          )}
        </div>
        <Button
          variant="ghost"
          size="icon"
          className="h-6 w-6 shrink-0 p-0"
          onClick={handleCopy}
          title={copied ? "Copied!" : "Copy"}
          aria-label={copied ? "Copied" : "Copy code"}
        >
          {copied ? (
            <Check className="h-3.5 w-3.5 text-green-500" />
          ) : (
            <Copy className="h-3.5 w-3.5 text-muted-foreground" />
          )}
        </Button>
      </div>
      <pre className="overflow-x-auto bg-background/70 py-3 text-xs leading-5">
        <HighlightedCode
          key={`${current.language ?? "text"}-${current.code}`}
          code={current.code}
          language={current.language}
        />
      </pre>
    </div>
  );
}

function HighlightedCode({ code, language }: { code: string; language?: string }) {
  const [highlighted, setHighlighted] = useState<HighlightResult | null>(null);

  useEffect(() => {
    if (!language || language === "text") return;

    let active = true;

    void loadHighlighter()
      .then((highlighter) => {
        const shikiLanguage = language as Parameters<CodeHighlighter["supportsLanguage"]>[0];
        if (!highlighter.supportsLanguage(shikiLanguage)) return;

        const update = (result: HighlightResult) => {
          if (active) setHighlighted(result);
        };
        const result = highlighter.highlight(
          { code, language: shikiLanguage, themes: highlighter.getThemes() },
          update,
        );
        if (result) update(result);
      })
      .catch(() => {
        // Plain code remains readable if the lazy-loaded highlighter is unavailable.
      });

    return () => {
      active = false;
    };
  }, [code, language]);

  if (!highlighted) {
    return (
      <code className="table min-w-full w-max px-3" data-language={language ?? "text"}>
        {code.split("\n").map((line, lineIndex) => (
          <CodeLineNumber key={lineIndex} line={lineIndex + 1}>
            {line || " "}
          </CodeLineNumber>
        ))}
      </code>
    );
  }

  return (
    <code className="table min-w-full w-max px-3" data-language={language ?? "text"}>
      {highlighted.tokens.map((line, lineIndex) => (
        <CodeLineNumber key={lineIndex} line={lineIndex + 1}>
          {line.length === 0 ? " " : null}
          {line.map((token, tokenIndex) => {
            const { color, ...htmlStyle } = token.htmlStyle ?? {};
            const style = {
              ...htmlStyle,
              "--code-token-light": color ?? token.color ?? "inherit",
            } as CSSProperties;

            return (
              <span
                className="text-[var(--code-token-light)] dark:text-[var(--shiki-dark,var(--code-token-light))]"
                key={`${tokenIndex}-${token.offset ?? 0}`}
                style={style}
              >
                {token.content || " "}
              </span>
            );
          })}
        </CodeLineNumber>
      ))}
    </code>
  );
}

function CodeLineNumber({ children, line }: { children: ReactNode; line: number }) {
  return (
    <span className="table-row">
      <span
        aria-hidden="true"
        className="hidden select-none pr-4 text-right text-muted-foreground/60 sm:table-cell"
      >
        {line}
      </span>
      <span className="table-cell pr-3">{children}</span>
    </span>
  );
}
