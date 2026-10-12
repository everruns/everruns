// A web search tool result's people-only details with a fallback (native skipped,
// Brave rate limited, free search answered), shaped like
// crates/integrations/src/web_search/tool.rs writes them. Shared by tests.

export const BRAVE_ACCOUNT_ID = "00000000-0000-0000-0000-000000000007";

export const fallbackDetails = {
  kind: "web_search",
  query: "tokio-console 0.2 release",
  setting: { provider: "org_default", account_id: null, fallback: null },
  resolved: { provider: "auto", account_id: null, fallback: "next" },
  plan: ["native", `account:${BRAVE_ACCOUNT_ID}`, "free"],
  ok: true,
  attempts: [
    {
      provider: "native",
      label: "Model provider search",
      ok: false,
      duration_ms: 0,
      failure_kind: "not_configured",
      message: "This model has no search of its own",
    },
    {
      provider: `account:${BRAVE_ACCOUNT_ID}`,
      label: "Brave Search",
      ok: false,
      duration_ms: 200,
      failure_kind: "rate_limited",
      message: "Brave Search returned 429",
      exchanges: [
        { method: "GET", url: "https://api.search.brave.com/res/v1/web/search?q=x", status: 429 },
      ],
    },
    {
      provider: "free",
      label: "Free search",
      ok: true,
      duration_ms: 1100,
      result_count: 2,
      exchanges: [
        {
          method: "GET",
          url: "https://html.duckduckgo.com/html/?q=tokio-console+0.2+release",
          status: 200,
          body: '<div class="result">tokio-rs/console</div>',
        },
      ],
    },
  ],
  results: [
    {
      id: "w1",
      title: "tokio-rs/console · Release v0.2.0",
      url: "https://github.com/tokio-rs/console/releases",
      site: "github.com",
    },
    {
      id: "w2",
      title: "Announcing tokio-console 0.2",
      url: "https://tokio.rs/blog/console",
      site: "tokio.rs",
    },
  ],
  provider: "free",
  provider_label: "Free search",
};
