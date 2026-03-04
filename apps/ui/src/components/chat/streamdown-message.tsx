"use client";

/**
 * StreamdownMessage - Unified markdown renderer using Streamdown
 *
 * Replaces react-markdown with Streamdown for streaming-optimized rendering.
 * Handles incomplete markdown blocks during streaming gracefully.
 *
 * @see specs/markdown-messages.md
 */

import { Streamdown, type StreamdownProps } from "streamdown";
import { code } from "@streamdown/code";
import remarkGfm from "remark-gfm";
import remarkGithubAlerts from "remark-github-blockquote-alert";
import { cn } from "@/lib/utils";
import type { ComponentType, AnchorHTMLAttributes } from "react";

/** Opens all markdown links in a new tab with noopener noreferrer. */
const ExternalLink: ComponentType<AnchorHTMLAttributes<HTMLAnchorElement>> = ({
  children,
  ...props
}) => (
  <a {...props} target="_blank" rel="noopener noreferrer">
    {children}
  </a>
);

const markdownComponents = { a: ExternalLink };

// Streamdown styling to match the design system
const streamdownStyles = `
  .streamdown h1 { font-size: 1.5em; font-weight: 700; margin: 1em 0 0.5em; }
  .streamdown h2 { font-size: 1.25em; font-weight: 600; margin: 1em 0 0.5em; }
  .streamdown h3 { font-size: 1.1em; font-weight: 600; margin: 1em 0 0.5em; }
  .streamdown p { margin: 0.5em 0; }
  .streamdown ul, .streamdown ol { margin: 0.5em 0; padding-left: 1.5em; }
  .streamdown li { margin: 0.25em 0; }
  .streamdown code:not(pre code) { background: var(--color-muted); padding: 0.2em 0.4em; font-size: 0.9em; }
  .streamdown pre { background: var(--color-muted); padding: 1em; overflow-x: auto; margin: 0.5em 0; }
  .streamdown pre code { background: none; padding: 0; }
  .streamdown blockquote { border-left: 3px solid var(--color-border); padding-left: 1em; margin: 0.5em 0; color: var(--color-muted-foreground); }
  .streamdown hr { border: none; border-top: 1px solid var(--color-border); margin: 1em 0; }
  .streamdown a { color: var(--color-primary); text-decoration: underline; }
  .streamdown strong { font-weight: 600; }
  .streamdown em { font-style: italic; }
  .streamdown table { border-collapse: collapse; width: 100%; margin: 0.5em 0; }
  .streamdown th, .streamdown td { border: 1px solid var(--color-border); padding: 0.5em; text-align: left; }
  .streamdown th { background: var(--color-muted); font-weight: 600; }
  .streamdown img { max-width: 100%; height: auto; }
  .streamdown [data-streamdown-code] { border-radius: 0; }

  /* GitHub-style alerts */
  .streamdown .markdown-alert { padding: 0.5em 1em; margin: 0.5em 0; border-left: 4px solid; }
  .streamdown .markdown-alert-title { display: flex; align-items: center; gap: 0.5em; font-weight: 600; margin-bottom: 0.25em; }
  .streamdown .markdown-alert-title svg { width: 1em; height: 1em; }

  .streamdown .markdown-alert-note { border-color: #0969da; background: rgba(9, 105, 218, 0.1); }
  .streamdown .markdown-alert-note .markdown-alert-title { color: #0969da; }

  .streamdown .markdown-alert-tip { border-color: #1a7f37; background: rgba(26, 127, 55, 0.1); }
  .streamdown .markdown-alert-tip .markdown-alert-title { color: #1a7f37; }

  .streamdown .markdown-alert-important { border-color: #8250df; background: rgba(130, 80, 223, 0.1); }
  .streamdown .markdown-alert-important .markdown-alert-title { color: #8250df; }

  .streamdown .markdown-alert-warning { border-color: #9a6700; background: rgba(154, 103, 0, 0.1); }
  .streamdown .markdown-alert-warning .markdown-alert-title { color: #9a6700; }

  .streamdown .markdown-alert-caution { border-color: #cf222e; background: rgba(207, 34, 46, 0.1); }
  .streamdown .markdown-alert-caution .markdown-alert-title { color: #cf222e; }
`;

export interface StreamdownMessageProps {
  children: string;
  /** Whether content is actively streaming */
  isAnimating?: boolean;
  /** Enable syntax highlighting for code blocks */
  enableCodeHighlighting?: boolean;
  /** Additional CSS class */
  className?: string;
  /** Variant for different display contexts */
  variant?: "default" | "compact" | "inline";
}

/**
 * Unified markdown renderer with streaming support.
 *
 * Uses Streamdown for optimized LLM output rendering:
 * - Handles incomplete/unterminated markdown blocks during streaming
 * - Memoized rendering for performance
 * - Built-in GFM support (tables, task lists, strikethrough)
 * - GitHub-style alerts ([!NOTE], [!TIP], [!IMPORTANT], [!WARNING], [!CAUTION])
 * - Optional Shiki-based syntax highlighting
 */
export function StreamdownMessage({
  children,
  isAnimating = false,
  enableCodeHighlighting = true,
  className,
  variant = "default",
}: StreamdownMessageProps) {
  // Build plugins
  const plugins: StreamdownProps["plugins"] = {};
  if (enableCodeHighlighting) {
    plugins.code = code;
  }

  return (
    <>
      <style>{streamdownStyles}</style>
      <div
        className={cn(
          "streamdown text-sm",
          variant === "default" && "bg-muted p-4",
          variant === "compact" && "bg-transparent p-0",
          variant === "inline" && "bg-transparent p-0",
          className,
        )}
      >
        <Streamdown
          plugins={plugins}
          isAnimating={isAnimating}
          remarkPlugins={[remarkGfm, remarkGithubAlerts]}
          components={markdownComponents}
        >
          {children}
        </Streamdown>
      </div>
    </>
  );
}

/**
 * Inline variant for descriptions - supports basic formatting
 * but without the container styling. Alias for variant="inline".
 */
export function InlineStreamdownMessage({
  children,
  className,
  isAnimating = false,
}: {
  children: string;
  className?: string;
  isAnimating?: boolean;
}) {
  return (
    <StreamdownMessage
      variant="inline"
      className={className}
      isAnimating={isAnimating}
      enableCodeHighlighting={false}
    >
      {children}
    </StreamdownMessage>
  );
}
