"use client";
import { useState } from "react";
import { WidgetContent } from "@/components/chat/widget-content";
import samples from "../../../../generative-ui/tests/standard-samples.json";

// This catalog uses the production console adapter. Fixture prose is authored
// test content; component schemas and the agent prompt remain generated.
export default function GenerativeUIDevPage() {
  const [sent, setSent] = useState("");
  if (process.env.NODE_ENV !== "development") return null;
  return (
    <main className="mx-auto max-w-3xl p-6" data-testid="generative-ui-catalog">
      <output aria-label="Last widget message">{sent}</output>
      {samples.map((sample, index) => {
        const surfaceId = `catalog-${index}`;
        const operations = [
          {
            version: "v1.0",
            createSurface: {
              surfaceId,
              catalogId: "copilotkit/standard@1",
              dataModel: sample.data,
              components: sample.components,
            },
          },
        ];
        return (
          <section key={sample.title} data-covers={sample.covers.join(",")}>
            <h2>{sample.title}</h2>
            <WidgetContent
              text={
                "```a2ui\n" +
                operations.map((operation) => JSON.stringify(operation)).join("\n") +
                "\n```"
              }
              onMessage={setSent}
            />
          </section>
        );
      })}
    </main>
  );
}
