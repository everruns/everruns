"use client";

/**
 * Dev page: OpenUI Blocks
 *
 * Smoke surface for the OpenUI renderer integration (knowledge/ui/openui.md).
 * Renders representative OpenUI Lang snippets through the production
 * <OpenUIBlock> + @openuidev/react-lang + @openuidev/react-ui pipeline so
 * upstream package bumps fail visibly during dev review.
 */

import { OpenUIBlock } from "@/components/chat/openui-renderer";
import { DevPageShell } from "@/app/dev/_components/dev-page-shell";

const SAMPLES: { label: string; code: string }[] = [
  {
    label: "Card with text",
    code: `root = Card([Text("Hello from OpenUI")])`,
  },
  {
    label: "Card with header and bar chart",
    code: `root = Card([header, chart])
header = CardHeader("Monthly Revenue", "Q1 2024")
chart = BarChart(labels, [series1])
labels = ["Jan", "Feb", "Mar"]
series1 = Series("Revenue", [45000, 52000, 61000])`,
  },
  {
    label: "Malformed input (should hit error boundary fallback)",
    code: `this is not valid openui lang $$$`,
  },
];

export default function OpenUIBlocksDevPage() {
  return (
    <DevPageShell
      eyebrow="OpenUI Blocks"
      title="OpenUI Lang renderer"
      description="Renders sample OpenUI Lang snippets through the production OpenUIBlock. Use as a visual smoke check after bumping @openuidev/react-lang or @openuidev/react-ui."
      widthClassName="max-w-3xl"
    >
      <div className="space-y-10">
        {SAMPLES.map((sample) => (
          <section key={sample.label} className="space-y-3">
            <h2 className="text-sm font-medium text-foreground">{sample.label}</h2>
            <pre className="overflow-x-auto border border-dashed border-border bg-muted/30 p-3 text-xs text-muted-foreground">
              <code>{sample.code}</code>
            </pre>
            <OpenUIBlock code={sample.code} />
          </section>
        ))}
      </div>
    </DevPageShell>
  );
}
