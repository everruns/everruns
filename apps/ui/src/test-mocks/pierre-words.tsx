/**
 * Jest stand-in for `@pierre/words`. The real renderer is WebAssembly loaded as ESM, which
 * jsdom cannot run; tests check what the console hands Words (text, hooks, plugins), and
 * the hooks themselves are pure string functions tested directly.
 */
import type { ReactNode } from "react";

export let lastMarkdownProps: Record<string, unknown> = {};

export function Markdown(props: { children?: ReactNode } & Record<string, unknown>) {
  lastMarkdownProps = props;
  return <div data-testid="words-markdown">{props.children}</div>;
}

export function createHighlightsPlugin(options: unknown) {
  return { name: "code", options };
}

export function createMermaidPlugin(renderer: unknown, options: unknown) {
  return { name: "mermaid", renderer, options };
}
