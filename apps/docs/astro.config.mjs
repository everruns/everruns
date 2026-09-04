import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import remarkGfm from "remark-gfm";
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";
import starlightSidebarTopics from "starlight-sidebar-topics";
import starlightLlmsTxt from "starlight-llms-txt";
import starlightLinksValidator from "starlight-links-validator";
import apiSidebarFix from "./plugins/api-sidebar-fix.ts";
import sitemapEnhance from "./integrations/sitemap-enhance.mjs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

// https://astro.build/config
export default defineConfig({
  site: "https://docs.everruns.com",
  trailingSlash: "always",
  // Register remark-gfm explicitly so GFM (tables, etc.) applies to `.mdx`
  // pages too. Starlight auto-adds `@astrojs/mdx` with extendMarkdownConfig,
  // which inherits `markdown.remarkPlugins` — but Astro's default `gfm: true`
  // flag alone does not reach the MDX pipeline, so `.mdx` tables silently
  // render as paragraphs. Listing the plugin here fixes every `.mdx` page.
  markdown: {
    remarkPlugins: [remarkGfm],
  },
  redirects: {
    // virtual_bash capability renamed to bashkit_shell
    "/capabilities/virtual-bash/": "/capabilities/bashkit-shell/",
    // generic-tool-search page renamed to tool-search
    "/capabilities/generic-tool-search/": "/capabilities/tool-search/",
    // Application users now enter through the Framework section. Keep the
    // former runtime, migration, and embedding routes durable for inbound links.
    "/features/runtime/": "/framework/custom-backends/",
    "/framework/runtime-compatibility/": "/framework/custom-backends/",
    "/advanced/embedding-everruns/": "/framework/custom-backends/",
  },
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
        {
          find: /^@docs-components\/(.*)$/,
          replacement: `${path.resolve(__dirname, "src/components")}/$1`,
        },
      ],
    },
  },
  integrations: [
    starlight({
      expressiveCode: {
        themes: ["github-light", "github-dark"],
      },
      title: "Everruns",
      description:
        "Build in-process agents with the Everruns Framework or deploy and operate the durable Everruns Platform.",
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
        Header: "./src/components/Header.astro",
        Search: "./src/components/Search.astro",
        Footer: "./src/components/Footer.astro",
        TableOfContents: "./src/components/TableOfContents.astro",
        MobileTableOfContents: "./src/components/MobileTableOfContents.astro",
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
                    { label: "Use in AI Tools", slug: "getting-started/use-in-ai-tools" },
                    { label: "Architecture", slug: "getting-started/architecture" },
                  ],
                },
                {
                  label: "Features",
                  items: [{ autogenerate: { directory: "features" } }],
                },
                {
                  label: "Advanced",
                  items: [{ autogenerate: { directory: "advanced" } }],
                },
              ],
            },
            {
              label: "Framework",
              link: "/framework/",
              icon: "rocket",
              items: [
                {
                  label: "Start",
                  items: [
                    { label: "Overview", slug: "framework" },
                    { label: "Quickstart", slug: "framework/quickstart" },
                    { label: "Architecture", slug: "framework/architecture" },
                  ],
                },
                {
                  label: "Core APIs",
                  items: [
                    { label: "Agents", slug: "framework/agents" },
                    { label: "Workspaces and Environments", slug: "framework/workspaces-and-environments" },
                    { label: "Workspace Security", slug: "framework/workspace-security" },
                    { label: "Models and Providers", slug: "framework/models-and-providers" },
                    { label: "Tools and Macros", slug: "framework/tools-and-macros" },
                    { label: "Sessions", slug: "framework/sessions" },
                    { label: "Session Work and Wakes", slug: "framework/background-work" },
                    { label: "Session History", slug: "framework/session-history" },
                    { label: "Events and Cancellation", slug: "framework/events-and-cancellation" },
                    { label: "Canonical Events", slug: "framework/canonical-events" },
                    { label: "Lifecycle Hooks", slug: "framework/lifecycle-hooks" },
                    { label: "Persistence", slug: "framework/persistence" },
                  ],
                },
                {
                  label: "Extend",
                  items: [
                    { label: "Advanced Capabilities", slug: "framework/advanced-capabilities" },
                    { label: "Capability Integrations", slug: "framework/capability-integrations" },
                    { label: "Portable vs Hosted", slug: "framework/capability-boundaries" },
                    { label: "Custom Providers", slug: "framework/custom-providers" },
                    { label: "Custom Backends", slug: "framework/custom-backends" },
                    { label: "Testing and Simulation", slug: "framework/testing-and-simulation" },
                  ],
                },
                {
                  label: "Examples",
                  items: [
                    { label: "Overview", slug: "framework/examples" },
                    { label: "Support Agent", slug: "framework/examples/support-agent" },
                    { label: "Everruns Support Agent", slug: "framework/examples/everruns-support-agent" },
                    { label: "Coding Review Agent", slug: "framework/examples/coding-review-agent" },
                    { label: "Research Agent", slug: "framework/examples/research-agent" },
                    { label: "Incident Commander Agent", slug: "framework/examples/incident-commander-agent" },
                  ],
                },
              ],
            },
            {
              label: "Built-ins",
              link: "/built-ins/",
              icon: "puzzle",
              items: [
                {
                  label: "Harnesses",
                  items: [{ autogenerate: { directory: "built-ins/harnesses" } }],
                },
                {
                  label: "Capabilities",
                  // Grouped to mirror the category taxonomy in
                  // docs/capabilities/index.md. Keep the two in sync when
                  // adding or recategorizing a capability.
                  items: [
                    { label: "Overview", slug: "capabilities" },
                    {
                      label: "Core",
                      collapsed: true,
                      items: [
                        { label: "File System", slug: "capabilities/file-system" },
                        { label: "Bashkit Shell", slug: "capabilities/bashkit-shell" },
                        { label: "Session", slug: "capabilities/session" },
                        { label: "Session Storage", slug: "capabilities/session-storage" },
                        { label: "Web Fetch", slug: "capabilities/web-fetch" },
                        { label: "Current Time", slug: "capabilities/current-time" },
                        { label: "Message Metadata", slug: "capabilities/message-metadata" },
                        { label: "Task Management", slug: "capabilities/task-management" },
                        { label: "Schedules", slug: "capabilities/session-schedules" },
                        {
                          label: "Auto-Continue After Usage Limit",
                          slug: "capabilities/usage-limit-auto-continue",
                        },
                        { label: "Sub Agents", slug: "capabilities/sub-agents" },
                        { label: "AGENTS.md", slug: "capabilities/agent-instructions" },
                        { label: "Agent Skills", slug: "capabilities/agent-skills" },
                      ],
                    },
                    {
                      label: "Sandboxes",
                      collapsed: true,
                      items: [
                        { label: "Daytona", slug: "capabilities/daytona" },
                        { label: "E2B", slug: "capabilities/e2b" },
                        { label: "Docker Container", slug: "capabilities/docker" },
                      ],
                    },
                    {
                      label: "Browser",
                      collapsed: true,
                      items: [
                        { label: "Browserless", slug: "capabilities/browserless" },
                      ],
                    },
                    {
                      label: "Data",
                      collapsed: true,
                      items: [
                        { label: "SQL Database", slug: "capabilities/sql-database" },
                        { label: "Retrieval Citations", slug: "capabilities/citation-retrieval" },
                        { label: "Citation Verification", slug: "capabilities/citation-verification" },
                      ],
                    },
                    {
                      label: "Media",
                      collapsed: true,
                      items: [
                        { label: "OpenAI Image Generation", slug: "capabilities/openai-image-generation" },
                      ],
                    },
                    {
                      label: "Tools",
                      collapsed: true,
                      items: [
                        { label: "OpenRouter Server Tools", slug: "capabilities/openrouter-server-tools" },
                      ],
                    },
                    {
                      label: "Integrations",
                      collapsed: true,
                      items: [
                        { label: "GitHub Scout", slug: "capabilities/github-scout" },
                      ],
                    },
                    {
                      label: "Platform",
                      collapsed: true,
                      items: [
                        { label: "Platform", slug: "capabilities/platform" },
                        { label: "Platform Management", slug: "capabilities/platform-management" },
                      ],
                    },
                    {
                      label: "Optimization",
                      collapsed: true,
                      items: [
                        { label: "Infinity Context", slug: "capabilities/infinity-context" },
                        { label: "Auto Tool Search", slug: "capabilities/auto-tool-search" },
                        { label: "OpenAI Tool Search", slug: "capabilities/openai-tool-search" },
                        { label: "Claude Tool Search", slug: "capabilities/claude-tool-search" },
                        { label: "Tool Search", slug: "capabilities/tool-search" },
                        { label: "Budgeting", slug: "capabilities/budgeting" },
                        { label: "Self-Budget", slug: "capabilities/self-budget" },
                        { label: "Parallel Tool Calls", slug: "capabilities/parallel-tool-calls" },
                      ],
                    },
                    {
                      label: "Safety",
                      collapsed: true,
                      items: [
                        { label: "Guardrails", slug: "capabilities/guardrails" },
                        { label: "Prompt Canary Guardrail", slug: "capabilities/prompt-canary-guardrail" },
                        { label: "Tool Call Repair", slug: "capabilities/tool-call-repair" },
                      ],
                    },
                    {
                      label: "Automation",
                      collapsed: true,
                      items: [
                        { label: "User Hooks", slug: "capabilities/user-hooks" },
                      ],
                    },
                    {
                      label: "Demo",
                      collapsed: true,
                      items: [
                        { label: "Fake Warehouse", slug: "capabilities/fake-warehouse" },
                        { label: "Fake AWS", slug: "capabilities/fake-aws" },
                        { label: "Fake CRM", slug: "capabilities/fake-crm" },
                      ],
                    },
                  ],
                },
              ],
            },
            {
              label: "Integrations",
              link: "/integrations/",
              icon: "laptop",
              items: [
                { label: "Overview", slug: "integrations" },
                // Grouped to mirror the category taxonomy in
                // docs/integrations/index.md. Keep the two in sync when adding
                // or recategorizing an integration. Sidebar glyphs for each
                // entry live in src/styles/custom.css, keyed by href.
                {
                  label: "Sandboxes & execution",
                  items: [
                    { label: "Daytona", slug: "integrations/daytona" },
                    { label: "E2B", slug: "integrations/e2b" },
                    { label: "Container Sandbox", slug: "integrations/container-sandbox" },
                    { label: "Sprites", slug: "integrations/sprites" },
                    { label: "Cursor", slug: "integrations/cursor" },
                  ],
                },
                {
                  label: "Browser & web",
                  items: [
                    { label: "Browserless", slug: "integrations/browserless" },
                    { label: "Brave Search", slug: "integrations/brave-search" },
                    { label: "DuckDuckGo", slug: "integrations/duckduckgo" },
                    { label: "Parallel", slug: "integrations/parallel" },
                  ],
                },
                {
                  label: "Messaging",
                  items: [{ label: "Slack", slug: "integrations/slack" }],
                },
                {
                  label: "Credentials",
                  items: [
                    { label: "Secure MCP Credentials", slug: "integrations/mcp-credentials" },
                  ],
                },
                {
                  label: "Discovery",
                  items: [{ label: "ARD", slug: "integrations/ard" }],
                },
                {
                  label: "Providers",
                  items: [{ autogenerate: { directory: "providers" } }],
                },
                {
                  label: "Observability",
                  items: [{ autogenerate: { directory: "observability" } }],
                },
                {
                  label: "Ecosystem",
                  items: [{ autogenerate: { directory: "ecosystem" } }],
                },
              ],
            },
            {
              label: "Tutorials",
              link: "/tutorials/run-an-agent/",
              icon: "rocket",
              items: [
                {
                  label: "Tutorials",
                  items: [
                    { label: "Run an Agent", slug: "tutorials/run-an-agent" },
                    { label: "Build your first agent", slug: "tutorials/building-agents-using-sdk" },
                  ],
                },
                {
                  label: "How-to guides",
                  items: [{ autogenerate: { directory: "how-to" } }],
                },
              ],
            },
            {
              label: "Explanation",
              link: "/explanation/",
              icon: "information",
              items: [
                {
                  label: "Explanation",
                  items: [{ autogenerate: { directory: "explanation" } }],
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
                  label: "How-to: Operate Everruns",
                  items: [
                    { label: "Environment Variables", slug: "sre/environment-variables" },
                    { label: "Admin Container", slug: "sre/admin-container" },
                    {
                      label: "Runbooks",
                      items: [{ autogenerate: { directory: "sre/runbooks" } }],
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
        // Generate /llms.txt, /llms-full.txt and /llms-small.txt so AI tools
        // can ingest the docs as clean Markdown. Pairs with the
        // "Use in AI Tools" guide and the AI-crawler allowlist in robots.txt.
        starlightLlmsTxt({
          projectName: "Everruns",
          description:
            "Everruns is a durable agentic harness engine for AI agents. " +
            "These docs cover deploying, configuring, and building agent " +
            "applications with the API.",
          // Keep the generated llms-full.txt focused on prose docs:
          // - The auto-generated OpenAPI reference is large and already
          //   available as a machine-readable schema at /api/openapi.json.
          // - The notebook-backed tutorial renders as a blob of Jupyter HTML
          //   via <NotebookDoc> (whose route lookup also can't resolve under
          //   the /llms-*.txt routes); the .ipynb source is linked in its
          //   `github` frontmatter for anyone who wants the runnable version.
          exclude: ["api/**", "tutorials/run-an-agent"],
        }),
        // Validate internal links and hash anchors at build time so broken
        // links fail CI instead of shipping. Runs last to see final routes.
        starlightLinksValidator({
          // The OpenAPI reference pages are generated by starlight-openapi at
          // build time and validated by that plugin, not authored here.
          exclude: ["/api/**"],
          // The Docker Compose guide documents the local stack's URLs
          // (http://localhost:9300/...) as example endpoints, not as site
          // links to follow. Don't treat those as broken links.
          errorOnLocalLinks: false,
        }),
      ],
      editLink: {
        baseUrl: "https://github.com/everruns/everruns/edit/main/apps/docs/",
      },
      lastUpdated: true,
    }),
    sitemapEnhance(),
  ],
});
