import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";

// https://astro.build/config
export default defineConfig({
  site: "https://docs.everruns.com",
  integrations: [
    starlight({
      title: "Everruns",
      logo: {
        src: "./src/assets/logo.svg",
      },
      favicon: "/favicon.svg",
      social: {
        github: "https://github.com/everruns/everruns",
      },
      customCss: ["./src/styles/custom.css"],
      sidebar: [
        {
          label: "Getting Started",
          items: [{ label: "Introduction", slug: "getting-started/introduction" }],
        },
        {
          label: "Features",
          autogenerate: { directory: "features" },
        },
        {
          label: "SRE Guide",
          items: [
            { label: "Environment Variables", slug: "sre/environment-variables" },
            { label: "Admin Container", slug: "sre/admin-container" },
            {
              label: "Runbooks",
              autogenerate: { directory: "sre/runbooks" },
            },
          ],
        },
        {
          label: "API Reference",
          items: [{ label: "Overview", slug: "api/overview" }],
        },
      ],
      editLink: {
        baseUrl: "https://github.com/everruns/everruns/edit/main/apps/docs/",
      },
      lastUpdated: true,
    }),
  ],
});
