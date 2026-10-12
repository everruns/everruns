// Prefixes root-relative links in docs content with the site base (`/docs`).
//
// Decision: the docs are served at everruns.com/docs/, but authors keep
// writing site-root links (`[x](/capabilities/web-search/)`, `<LinkCard
// href="/framework/">`). Astro only applies `base` to the URLs it generates
// itself, not to links inside Markdown/MDX, so this plugin adds it at build
// time. Content stays portable, and the links validator sees the real URLs.

import { withBase } from "../site.config.mjs";

const LINK_ATTRIBUTES = new Set(["href", "src", "poster"]);

type MdxAttribute = { type: string; name?: string; value?: unknown };

export default function rehypeBaseLinks({ base }: { base: string }) {
  const visit = (node: any) => {
    if (node.type === "element" && node.properties) {
      for (const name of LINK_ATTRIBUTES) {
        const value = node.properties[name];
        if (typeof value === "string")
          node.properties[name] = withBase(value, base);
      }
    } else if (node.type === "raw" && typeof node.value === "string") {
      // Inline HTML in .md files (`<img src="/images/...">`) stays a raw string.
      node.value = node.value.replace(
        /\b(href|src|poster)=(["'])(\/[^"']*)\2/g,
        (_match: string, name: string, quote: string, url: string) =>
          `${name}=${quote}${withBase(url, base)}${quote}`,
      );
    } else if (
      node.type === "mdxJsxFlowElement" ||
      node.type === "mdxJsxTextElement"
    ) {
      for (const attribute of (node.attributes ?? []) as MdxAttribute[]) {
        if (
          attribute.type === "mdxJsxAttribute" &&
          attribute.name &&
          LINK_ATTRIBUTES.has(attribute.name) &&
          typeof attribute.value === "string"
        ) {
          attribute.value = withBase(attribute.value, base);
        }
      }
    }
    for (const child of node.children ?? []) visit(child);
  };
  return (tree: unknown) => visit(tree);
}
