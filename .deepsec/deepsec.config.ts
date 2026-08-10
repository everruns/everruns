import { defineConfig } from "deepsec/config";

export default defineConfig({
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
