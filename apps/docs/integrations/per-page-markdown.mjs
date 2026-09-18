import { existsSync, readFileSync, readdirSync, unlinkSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const removeRedirectMarkdown = (directory) => {
  for (const entry of readdirSync(directory, { withFileTypes: true })) {
    if (!entry.isDirectory()) continue;
    const child = path.join(directory, entry.name);
    removeRedirectMarkdown(child);
    const html = path.join(child, "index.html");
    const markdown = path.join(child, "index.md");
    if (
      existsSync(html) &&
      existsSync(markdown) &&
      readFileSync(html, "utf8").includes('<meta http-equiv="refresh"')
    ) {
      unlinkSync(markdown);
    }
  }
};

export default function perPageMarkdown() {
  return {
    name: "per-page-markdown",
    hooks: {
      "astro:config:setup"({ injectRoute }) {
        injectRoute({
          entrypoint: new URL("./per-page-markdown-route.ts", import.meta.url),
          pattern: "/[...slug].md",
          prerender: true,
        });
      },
      "astro:build:done"({ dir }) {
        removeRedirectMarkdown(fileURLToPath(dir));
      },
    },
  };
}
