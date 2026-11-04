// Mermaid is loaded only on pages with diagrams. The source stays visible if
// loading or parsing fails, including when JavaScript is unavailable.
let rendererPromise: Promise<typeof import("mermaid")["default"]> | undefined;

function getRenderer() {
  rendererPromise ??= import("mermaid").then(({ default: mermaid }) => {
    mermaid.initialize({
      startOnLoad: false,
      securityLevel: "strict",
      // Mermaid 12 defaults to the ELK layout and the "neo" look; pin the
      // dagre layout and classic look this theme and flowchart config target.
      layout: "dagre",
      look: "classic",
      theme: "base",
      themeVariables: {
        background: "#FFFFFF",
        primaryColor: "#F5F5F5",
        primaryBorderColor: "#0A0A0A",
        primaryTextColor: "#0A0A0A",
        secondaryColor: "#FFFFFF",
        secondaryBorderColor: "#404040",
        secondaryTextColor: "#404040",
        tertiaryColor: "#FFFFFF",
        tertiaryBorderColor: "#A0A0A0",
        tertiaryTextColor: "#404040",
        lineColor: "#0A1636",
        textColor: "#0A0A0A",
        edgeLabelBackground: "#FFFFFF",
        fontFamily: '"Geist Variable", "Geist", system-ui, sans-serif',
        fontSize: "13px",
      },
      themeCSS: ".node rect, .node polygon, .cluster rect { rx: 0; ry: 0; stroke-width: 1px; } .edgePath .path { stroke-width: 1.5px; }",
      flowchart: { htmlLabels: false, curve: "linear", nodeSpacing: 50, rankSpacing: 65 },
    });
    return mermaid;
  }).catch((error) => {
    rendererPromise = undefined;
    throw error;
  });
  return rendererPromise;
}

let diagramId = 0;

async function renderDiagrams() {
  const figures = document.querySelectorAll<HTMLElement>("[data-mermaid-diagram]:not([data-mermaid-rendering]):not([data-mermaid-rendered])");
  if (figures.length === 0) return;

  for (const figure of figures) figure.dataset.mermaidRendering = "";

  try {
    const mermaid = await getRenderer();
    for (const figure of figures) {
      const source = figure.querySelector(".er-mermaid-source code")?.textContent;
      if (!source) continue;
      try {
        const { svg, bindFunctions } = await mermaid.render(`er-mermaid-${++diagramId}`, source);
        const parsed = new DOMParser().parseFromString(svg, "image/svg+xml");
        if (parsed.documentElement.nodeName !== "svg") throw new Error("Invalid Mermaid SVG");
        const rendered = document.importNode(parsed.documentElement, true);
        const viewBoxWidth = Number(rendered.getAttribute("viewBox")?.split(/\s+/)[2]);
        if (Number.isFinite(viewBoxWidth)) {
          // Preserve Mermaid's label size on wide diagrams, and keep narrow
          // diagrams readable without stretching them across the article.
          rendered.style.width = `${Math.max(viewBoxWidth, 420)}px`;
          if (viewBoxWidth < 420) rendered.style.maxWidth = "100%";
        }
        figure.replaceChildren(rendered);
        figure.dataset.mermaidRendered = "";
        bindFunctions?.(figure);
      } catch (error) {
        console.error("Could not render Mermaid diagram", error);
      }
    }
  } catch (error) {
    console.error("Could not load Mermaid", error);
  } finally {
    for (const figure of figures) delete figure.dataset.mermaidRendering;
  }
}

void renderDiagrams();
document.addEventListener("astro:page-load", () => void renderDiagrams());
