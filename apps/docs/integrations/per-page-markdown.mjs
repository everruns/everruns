export default function perPageMarkdown() {
  return {
    name: "per-page-markdown",
    hooks: {
      "astro:config:setup": ({ injectRoute }) => {
        const entrypoint = new URL("./per-page-markdown-route.ts", import.meta.url);
        injectRoute({
          pattern: "/index.md",
          entrypoint,
          prerender: true,
        });
        injectRoute({
          pattern: "/[...path]/index.md",
          entrypoint,
          prerender: true,
        });
      },
    },
  };
}
