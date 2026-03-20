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
import { code as baseCodePlugin } from "@streamdown/code";
import remarkGfm from "remark-gfm";
import remarkGithubAlerts from "remark-github-blockquote-alert";
import { cn } from "@/lib/utils";
import type { ComponentType, AnchorHTMLAttributes } from "react";
import "./streamdown-message.css";

// Wrap the default code plugin to skip "openui" language — OpenUI blocks are
// rendered by OpenUIBlock, but during streaming incomplete fences may reach Streamdown.
const code = {
  ...baseCodePlugin,
  // eslint-disable-next-line @typescript-eslint/no-explicit-any
  supportsLanguage: (lang: any) =>
    lang === "openui" ? false : baseCodePlugin.supportsLanguage(lang),
};

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

// Streamdown styles loaded from streamdown-message.css

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
