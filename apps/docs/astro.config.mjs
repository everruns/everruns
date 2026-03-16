import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";
import starlightSidebarTopics from "starlight-sidebar-topics";
import apiSidebarFix from "./plugins/api-sidebar-fix.ts";
import sitemapEnhance from "./integrations/sitemap-enhance.mjs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// https://astro.build/config
export default defineConfig({
  site: "https://docs.everruns.com",
  trailingSlash: "always",
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
      description:
        "Documentation for Everruns, a durable agentic harness engine for AI agents",
      routeMiddleware: "./src/routeData.ts",
      logo: {
        src: "./src/assets/logo.svg",
        alt: "Everruns",
      },
      favicon: "/favicon.svg",
      head: [
        {
          tag: "meta",
          attrs: {
            name: "msvalidate.01",
            content: "CA0AE96A84D6EB1E18A00BA8F0F8C70A",
          },
        },
        {
          tag: "meta",
          attrs: {
            name: "google-site-verification",
            content: "xzTq83UYKYRkoPnxWzaF5uS1gJ6wVoY_cP5oEBRe9IM",
          },
        },
        // Twitter / Open Graph social card
        {
          tag: "meta",
          attrs: { name: "twitter:card", content: "summary_large_image" },
        },
        {
          tag: "meta",
          attrs: { name: "twitter:site", content: "@everrunshq" },
        },
        {
          tag: "meta",
          attrs: {
            property: "og:image",
            content: "https://docs.everruns.com/og-image.png",
          },
        },
        {
          tag: "meta",
          attrs: { property: "og:image:width", content: "1200" },
        },
        {
          tag: "meta",
          attrs: { property: "og:image:height", content: "630" },
        },
        {
          tag: "meta",
          attrs: { property: "og:image:alt", content: "Everruns — Durable Agentic Harness Engine" },
        },
      ],
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/everruns/everruns" },
      ],
      components: {
        Head: "./src/components/Head.astro",
        Header: "./src/components/Header.astro",
      },
      customCss: ["./src/styles/custom.css"],
      plugins: [
        starlightOpenAPI([
          {
            base: "api",
            label: "API Reference",
            schema: "../../docs/api/openapi.json",
          },
        ]),
        starlightSidebarTopics(
          [
            {
              label: "Get Started",
              link: "/getting-started/introduction/",
              icon: "open-book",
              items: [
                {
                  label: "Getting Started",
                  items: [
                    { label: "Introduction", slug: "getting-started/introduction" },
                    { label: "Concepts", slug: "getting-started/concepts" },
                    { label: "Docker Compose", slug: "getting-started/docker-compose" },
                    { label: "Architecture", slug: "getting-started/architecture" },
                  ],
                },
                {
                  label: "Features",
                  autogenerate: { directory: "features" },
                },
                {
                  label: "Advanced",
                  autogenerate: { directory: "advanced" },
                },
              ],
            },
            {
              label: "Capabilities",
              link: "/capabilities/",
              icon: "puzzle",
              items: [
                {
                  label: "Capabilities",
                  autogenerate: { directory: "capabilities" },
                },
              ],
            },
            {
              label: "Integrations",
              link: "/integrations/daytona/",
              icon: "laptop",
              items: [
                {
                  label: "Integrations",
                  autogenerate: { directory: "integrations" },
                },
                {
                  label: "Observability",
                  autogenerate: { directory: "observability" },
                },
                {
                  label: "Ecosystem",
                  autogenerate: { directory: "ecosystem" },
                },
              ],
            },
            {
              label: "Tutorials",
              link: "/tutorials/building-agents-using-sdk/",
              icon: "rocket",
              items: [
                {
                  label: "Tutorials",
                  items: [
                    { label: "Building Agents Using the SDK", slug: "tutorials/building-agents-using-sdk" },
                  ],
                },
              ],
            },
            {
              label: "Reference",
              link: "/api/",
              icon: "information",
              id: "reference",
              items: [
                { label: "Event Reference", slug: "event-reference" },
                ...openAPISidebarGroups,
              ],
            },
            {
              label: "Operations",
              link: "/sre/environment-variables/",
              icon: "setting",
              items: [
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
              ],
            },
          ],
          {
            exclude: ["/", "/api/**"],
          },
        ),
        apiSidebarFix(),
      ],
      editLink: {
        baseUrl: "https://github.com/everruns/everruns/edit/main/apps/docs/",
      },
      lastUpdated: true,
    }),
    sitemapEnhance(),
  ],
});
