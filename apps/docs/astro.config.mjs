import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";

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
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/everruns/everruns" },
      ],
      customCss: ["./src/styles/custom.css"],
      plugins: [
        starlightOpenAPI([
          {
            base: "api",
            label: "API Reference",
            schema: "../../docs/api/openapi.json",
          },
        ]),
      ],
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
        // Auto-generated API Reference from OpenAPI spec
        ...openAPISidebarGroups,
      ],
      editLink: {
        baseUrl: "https://github.com/everruns/everruns/edit/main/apps/docs/",
      },
      lastUpdated: true,
    }),
  ],
});
