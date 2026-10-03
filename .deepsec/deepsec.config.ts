import { generatedMatchersPlugin } from "./generated-matchers.js";
import { defineConfig } from "deepsec/config";

export default defineConfig({
  defaultThinkingLevel: "xhigh", // <deepsec:default-thinking-level>
  defaultModel: "gpt-5.6-sol", // <deepsec:default-model>
  defaultAgent: "codex", // <deepsec:default-agent>
  ai: {"mode":"local","provider":"local"}, // <deepsec:model-route>
  plugins: [generatedMatchersPlugin],
  projects: [
    {
      id: "everruns",
      root: "..",
      priorityPaths: [
        "apps/ui/",
        "crates/server/",
        "crates/core/",
        "crates/worker/",
        "crates/host/",
        "integrations/",
      ],
    },
    // <deepsec:projects-insert-above>
  ],
});
