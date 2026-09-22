import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import remarkGfm from "remark-gfm";
import starlightOpenAPI, { openAPISidebarGroups } from "starlight-openapi";
import starlightSidebarTopics from "starlight-sidebar-topics";
import starlightLlmsTxt from "starlight-llms-txt";
import starlightLinksValidator from "starlight-links-validator";
import apiSidebarFix from "./plugins/api-sidebar-fix.ts";
import remarkStripRustHiddenLines from "./plugins/remark-strip-rust-hidden-lines.ts";
import perPageMarkdown from "./integrations/per-page-markdown.mjs";
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
    remarkPlugins: [
      remarkGfm,
      // Hide rustdoc `# ` boilerplate lines in Rust blocks. rustdoc hides
      // them; Starlight would otherwise render the `#` prefixes literally.
      remarkStripRustHiddenLines,
    ],
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
    "/framework/direct-classification/": "/framework/direct-decisions/",
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
                    { label: "Supported Providers", slug: "framework/supported-providers" },
                    { label: "Direct Model Calls", slug: "framework/direct-model-calls" },
                    { label: "Direct Decisions", slug: "framework/direct-decisions" },
                    { label: "Model Catalogs", slug: "framework/model-catalogs" },
                    { label: "Credentials", slug: "framework/credentials" },
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
                    { label: "Bashkit Repo Agent", slug: "framework/examples/bashkit-repo-agent" },
                    { label: "Host Shell Agent", slug: "framework/examples/host-shell-agent" },
                    { label: "Foreman", slug: "framework/examples/foreman-agent" },
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
                        { label: "Host Shell", slug: "capabilities/host-shell" },
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
                        { label: "Slack", slug: "capabilities/slack" },
                      ],
                    },
                    {
                      label: "Platform",
                      collapsed: true,
                      items: [
                        { label: "Platform", slug: "capabilities/platform" },
                        { label: "Platform Management (removed)", slug: "capabilities/platform-management" },
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
                  label: "Reasoning & judgment",
                  items: [{ label: "TypeSafe", slug: "integrations/typesafe" }],
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
        // Generate /llms.txt, /llms-full.txt, /llms-small.txt and the
        // per-topic /_llms-txt/<slug>.txt sets so AI tools can ingest the docs
        // as clean Markdown. Pairs with the "Read the docs as text" section of
        // the "Use in AI Tools" guide and the AI-crawler allowlist in
        // robots.txt. Requirements live in knowledge/ui/documentation.md.
        starlightLlmsTxt({
          projectName: "Everruns",
          description:
            "Everruns is a durable agentic harness engine built on Rust. " +
            "These docs cover deploying and operating the Platform, and " +
            "building agents with the Framework, SDKs, and REST API.",
          // Answers the first question a reader of llms.txt has: which of the
          // three ways to run Everruns am I looking at? Mirrors the "Choose how
          // you run Everruns" table in the repository README — keep the two in
          // sync. llmstxt.org allows no headings here, so this is prose only.
          details: [
            "Everruns is used in three ways, in increasing order of what it operates for you:",
            "",
            "- **Framework** — embed Everruns in a Rust application with the `everruns` crate. You own the process, deployment, integrations, and data path. Start at <https://docs.everruns.com/framework/quickstart/>.",
            "- **Self-hosted platform** — run the shared runtime in infrastructure you manage when you need a control plane, server, workers, UI, remote API, and durable execution. Start at <https://docs.everruns.com/getting-started/docker-compose/>.",
            "- **Hosted Everruns** — use the shared runtime and production operations without operating the platform yourself: <https://app.everruns.com>.",
            "",
            "The documentation sets below are organised the same way, so a reader with a narrow question can take one set instead of the complete text. Every page in them carries a `Source:` line with its canonical URL; cite that rather than the text file.",
            "",
            "The sets contain prose documentation only. The REST API is not among them: take its shapes from the OpenAPI schema linked under Optional.",
          ].join("\n"),
          // Per-topic subsets, served at /_llms-txt/<slug>.txt. The plugin
          // derives no page index from the sidebar, so without these llms.txt
          // offers nothing but the two whole-site dumps — and the complete text
          // is ~250k tokens, more than most readers want for one question.
          // Mirrors the sidebar topics above; keep the two in sync.
          customSets: [
            {
              label: "Framework",
              description:
                "build and run agents inside a Rust application with the everruns crate",
              paths: ["index", "framework/**"],
            },
            {
              label: "Getting Started",
              description:
                "platform concepts, deployment, and the feature surface of a running Everruns",
              paths: ["index", "getting-started/**", "features/**", "advanced/**"],
            },
            {
              label: "Built-ins",
              description: "the harness and capability catalog, with tools and parameters",
              paths: ["built-ins/**", "capabilities/**"],
            },
            {
              label: "Guides",
              description: "tutorials and task-oriented how-to guides",
              paths: ["tutorials/**", "how-to/**"],
            },
            {
              label: "Integrations",
              description:
                "model providers, sandboxes, browsers, messaging, and observability vendors",
              paths: [
                "integrations/**",
                "providers/**",
                "observability/**",
                "ecosystem/**",
              ],
            },
            {
              label: "Explanation",
              description: "architecture and design rationale",
              paths: ["explanation/**"],
            },
            {
              label: "Reference",
              description: "the event protocol; REST endpoints live in the OpenAPI schema",
              paths: ["event-reference"],
            },
            {
              label: "Operations",
              description: "environment variables, admin container, and SRE runbooks",
              paths: ["sre/**"],
            },
          ],
          // Secondary material a reader can skip, and the machine-readable
          // surfaces that are deliberately absent from the text sets.
          optionalLinks: [
            {
              label: "OpenAPI schema",
              url: "https://docs.everruns.com/api/openapi.json",
              description:
                "the REST API as OpenAPI 3.0 — authoritative request and response shapes",
            },
            {
              label: "REST API reference",
              url: "https://docs.everruns.com/api/",
              description: "the same endpoints as browsable pages",
            },
            {
              label: "Source repository",
              url: "https://github.com/everruns/everruns",
              description: "Rust workspace, examples, and contributor conventions",
            },
            {
              label: "everruns crate",
              url: "https://crates.io/crates/everruns",
              description: "the application-facing crate on crates.io",
            },
          ],
          // Starlight renders a sibling anchor link after every heading, whose
          // screen-reader text converts to `[Section titled "..."](#...)` — was
          // 1,325 lines and 8% of llms-full.txt before this filter. Removing
          // the <a> leaves the heading itself untouched.
          customSelectors: { all: ["a.sl-anchor-link"] },
          // Pages are otherwise separated by a blank line, which is
          // indistinguishable from a paragraph break; `# ` headings are no help
          // because shell comments inside code fences start the same way.
          pageSeparator: "\n\n---\n\n",
          // Keep every generated text output focused on prose docs:
          // - The auto-generated OpenAPI reference is large and already
          //   available as a machine-readable schema at /api/openapi.json,
          //   copied there from docs/api/openapi.json by scripts/copy-openapi.mjs.
          // - The notebook-backed tutorial renders as a blob of Jupyter HTML
          //   via <NotebookDoc> (whose route lookup also can't resolve under
          //   the /llms-*.txt routes); the .ipynb source is linked in its
          //   `github` frontmatter for anyone who wants the runnable version.
          exclude: ["api/**", "tutorials/run-an-agent"],
          // The abridged set drops vendor- and operator-specific long tails
          // that a reader asking how to build or run an agent does not need.
          // They stay in llms-full.txt and in the sets above, so nothing is
          // unreachable. Without this, "abridged" would mean nothing but
          // collapsed whitespace: 91% the size of the complete text.
          excludeSmall: [
            "sre/**",
            "providers/**",
            "observability/**",
            "ecosystem/**",
            "integrations/**",
            "event-reference",
            "capabilities/fake-*",
            "capabilities/platform-management",
          ],
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
    perPageMarkdown(),
    sitemapEnhance(),
  ],
});
