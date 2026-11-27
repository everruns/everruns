"use client";

/**
 * MarkdownMessage - the console's Markdown renderer, for chat messages and every other
 * place that shows Markdown written by agents or people.
 *
 * Decisions:
 * - Rendered with Words (`@pierre/words`, Apache-2.0), a streaming GFM renderer in
 *   WebAssembly. Measured against the Streamdown setup it replaced, it used about 3.5x
 *   less CPU while a reply streams and about 8x less memory; see
 *   `knowledge/ui/markdown-messages.md`.
 * - Words supports no remark/rehype plugins. GitHub alerts, link handling and citation
 *   markers are HTML hooks (`markdown-hooks.ts`); React decorations go through
 *   `MarkdownSlots`.
 * - Syntax highlighting (`@pierre/highlights`, every grammar) and Mermaid diagrams (Pele)
 *   load on first use, so a page without code or diagrams pays for neither. Until the
 *   highlighter arrives, code renders as plain text.
 * - Raw HTML: only the presentational tags in `ALLOWED_HTML` render, without attributes
 *   other than `details open`; Words drops every other tag, attribute and comment. Agents
 *   use `<details>` for long output, which GFM has no syntax for. `skipHtml` turns even
 *   these off for surfaces that must show text only.
 *
 * @see knowledge/ui/markdown-messages.md
 */

import { Markdown, type PluginConfig } from "@pierre/words/react";
import "@pierre/words/styles.css";
import { useEffect, useMemo, useState } from "react";
import { cn } from "@/lib/utils";
import type { CitationSource } from "@/components/chat/message-citations";
import { alertBlockquoteHook, createLinkHook } from "./markdown-hooks";
import { MarkdownSlots } from "./markdown-slots";
import "./markdown-message.css";

type CodePlugin = NonNullable<PluginConfig["code"]>;
type MermaidPlugin = NonNullable<PluginConfig["mermaid"]>;

let codePlugin: CodePlugin | null = null;
let codePluginLoading: Promise<CodePlugin> | null = null;
function loadCodePlugin(): Promise<CodePlugin> {
  codePluginLoading ??= Promise.all([
    import("@pierre/words/highlights"),
    import("@pierre/highlights/themes/github-light"),
    import("@pierre/highlights/themes/github-dark"),
  ]).then(([{ createHighlightsPlugin }, light, dark]) => {
    // `light-dark()` follows the `color-scheme` design-system.css sets for each theme.
    codePlugin = createHighlightsPlugin({
      themes: { light: light.default, dark: dark.default },
      defaultColor: "light-dark()",
    });
    return codePlugin;
  });
  return codePluginLoading;
}

let mermaidPlugin: MermaidPlugin | null = null;
let mermaidPluginLoading: Promise<MermaidPlugin> | null = null;
function loadMermaidPlugin(): Promise<MermaidPlugin> {
  mermaidPluginLoading ??= Promise.all([import("@pierre/words/mermaid"), import("pele")]).then(
    ([{ createMermaidPlugin }, pele]) => {
      // Diagrams are agent-written: no links or remote images inside them.
      mermaidPlugin = createMermaidPlugin(pele, { links: false, images: false });
      return mermaidPlugin;
    },
  );
  return mermaidPluginLoading;
}

const ALLOWED_HTML = {
  details: ["open"],
  summary: [],
  kbd: [],
  sub: [],
  sup: [],
  br: [],
} as const;

const MERMAID_FENCE = /^\s*(```|~~~)\s*mermaid\b/im;
const CODE_FENCE = /^\s*(```|~~~)/m;

/** Loads a plugin the first time the text needs it, and re-renders once it arrives. */
function useLazyPlugin<T>(
  needed: boolean,
  current: () => T | null,
  load: () => Promise<T>,
): T | undefined {
  const [plugin, setPlugin] = useState<T | null>(current);
  useEffect(() => {
    if (!needed || plugin) return;
    let live = true;
    load().then((loaded) => live && setPlugin(loaded));
    return () => {
      live = false;
    };
  }, [needed, plugin, load]);
  // Another message may have loaded it already; still pass it only where it is wanted.
  return needed ? (plugin ?? undefined) : undefined;
}

export interface MarkdownMessageProps {
  children: string;
  /** Whether content is actively streaming */
  isAnimating?: boolean;
  /** Enable syntax highlighting for code blocks */
  enableCodeHighlighting?: boolean;
  /** Additional CSS class */
  className?: string;
  /** Variant for different display contexts */
  variant?: "default" | "compact" | "inline";
  /**
   * When set, `#cite-<n>` links in the text render as inline citation chips
   * looked up in this map (keyed by 1-based source number).
   */
  citations?: Map<number, CitationSource>;
  /** Never render raw HTML from the text, not even allowlisted tags. */
  skipHtml?: boolean;
}

export function MarkdownMessage({
  children,
  isAnimating = false,
  enableCodeHighlighting = true,
  className,
  variant = "default",
  citations,
  skipHtml = false,
}: MarkdownMessageProps) {
  const [container, setContainer] = useState<HTMLDivElement | null>(null);
  const code = useLazyPlugin(
    enableCodeHighlighting && CODE_FENCE.test(children),
    () => codePlugin,
    loadCodePlugin,
  );
  const mermaid = useLazyPlugin(
    MERMAID_FENCE.test(children),
    () => mermaidPlugin,
    loadMermaidPlugin,
  );

  const plugins = useMemo<PluginConfig>(() => ({ code, mermaid }), [code, mermaid]);
  const citationKey = citations ? [...citations.keys()].join(",") : "";
  const components = useMemo(
    () => ({
      a: createLinkHook(citationKey ? new Set(citationKey.split(",").map(Number)) : undefined),
      blockquote: alertBlockquoteHook,
    }),
    [citationKey],
  );

  return (
    <div
      ref={setContainer}
      className={cn(
        "markdown-message text-sm",
        variant === "default" && "bg-muted p-4",
        variant === "compact" && "bg-transparent p-0",
        variant === "inline" && "bg-transparent p-0",
        className,
      )}
    >
      <Markdown
        isAnimating={isAnimating}
        plugins={plugins}
        components={components}
        allowedHtml={skipHtml ? undefined : ALLOWED_HTML}
      >
        {children}
      </Markdown>
      <MarkdownSlots container={container} citations={citations} />
    </div>
  );
}

/**
 * Inline variant for descriptions - supports basic formatting
 * but without the container styling. Alias for variant="inline".
 */
export function InlineMarkdownMessage({
  children,
  className,
  isAnimating = false,
  skipHtml,
}: {
  children: string;
  className?: string;
  isAnimating?: boolean;
  skipHtml?: boolean;
}) {
  return (
    <MarkdownMessage
      variant="inline"
      className={className}
      isAnimating={isAnimating}
      enableCodeHighlighting={false}
      skipHtml={skipHtml}
    >
      {children}
    </MarkdownMessage>
  );
}
