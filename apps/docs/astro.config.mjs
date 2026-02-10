import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";
import starlightClientMermaid from "@pasqal-io/starlight-client-mermaid";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// https://astro.build/config
export default defineConfig({
  site: "https://docs.everruns.com",
  vite: {
    resolve: {
      // Enable Starlight component imports from symlinked docs/ directory
      alias: [
        {
          find: /^@astrojs\/starlight\/components$/,
          replacement: path.resolve(
            __dirname,
            "node_modules/@astrojs/starlight/components.ts"
          ),
        },
      ],
    },
  },
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
        starlightClientMermaid(),
      ],
      sidebar: [
        {
          label: "Getting Started",
          items: [
            { label: "Introduction", slug: "getting-started/introduction" },
            { label: "Concepts", slug: "getting-started/concepts" },
            { label: "Docker Compose Quickstart", slug: "getting-started/docker-compose" },
            { label: "Architecture", slug: "getting-started/architecture" },
          ],
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
          label: "Observability",
          autogenerate: { directory: "observability" },
        },
        // Auto-generated API Reference from OpenAPI spec
        ...openAPISidebarGroups,
      ],
      editLink: {
        baseUrl: "https://github.com/everruns/everruns/blob/main/docs/",
      },
      lastUpdated: true,
    }),
  ],
});
