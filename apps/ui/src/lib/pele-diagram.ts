import type { DiagramPlugin } from "streamdown";

// Streamdown's diagram UI expects the Mermaid render interface. Pele returns
// the same SVG shape, so keep its zoom and error controls while changing the engine.
export const peleDiagram: DiagramPlugin = {
  name: "mermaid",
  type: "diagram",
  language: "mermaid",
  getMermaid: () => ({
    initialize: () => {},
    render: async (_id, source) => {
      const { render } = await import("pele");
      return { svg: render(source, { links: false, images: false }).svg };
    },
  }),
};
