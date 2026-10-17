// Keep Mermaid source in the HTML as a readable fallback. The docs client
// replaces it with a brand-styled SVG after Mermaid loads on diagram pages.
const escapeHtml = (value) =>
  value.replaceAll("&", "&amp;").replaceAll("<", "&lt;").replaceAll(">", "&gt;");

export default function remarkMermaidDiagrams() {
  return (tree) => {
    const visit = (node) => {
      for (const [index, child] of (node.children ?? []).entries()) {
        if (child.type === "code" && child.lang?.toLowerCase() === "mermaid") {
          node.children[index] = {
            type: "html",
            value: `<figure class="er-mermaid" data-mermaid-diagram><pre class="er-mermaid-source"><code>${escapeHtml(child.value)}</code></pre></figure>`,
          };
        } else {
          visit(child);
        }
      }
    };
    visit(tree);
  };
}
