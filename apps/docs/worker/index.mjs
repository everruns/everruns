// The docs Worker: serves the static docs build at everruns.com/docs/.
//
// Decision: the landing site owns everruns.com (a Custom Domain on the
// everruns-landing Worker); this Worker is attached to the more specific route
// `everruns.com/docs*`, which Cloudflare matches first. The two sites deploy
// independently and no request passes through both Workers.
//
// Beyond static assets it does one thing: content negotiation. Every page has
// a Markdown twin at `<page>/index.md` (integrations/per-page-markdown.mjs);
// an AI client asking for `text/markdown` on the page URL gets that twin, the
// same contract the landing site's middleware follows, so agents see one
// behavior across everruns.com.
import { BASE, SITE, redirectedPagePath } from "../site.config.mjs";

// The former docs host. This Worker also takes the route `docs.everruns.com/*`
// and sends every URL to the same path under everruns.com/docs, query kept. A
// zone redirect rule would do the same, but the deploy token can manage Worker
// routes, not rulesets, and this way the redirect is tested and versioned.
const OLD_HOST = "docs.everruns.com";

// Same lists as the landing site's src/middleware.ts: crawlers always get HTML,
// AI clients get Markdown when they ask for it or identify themselves.
const AI_USER_AGENT_PATTERNS = [
  "claude",
  "anthropic",
  "cursor",
  "copilot",
  "openai",
  "gpt",
  "chatgpt",
  "ai-agent",
  "llm",
  "langchain",
  "autogpt",
];
const SEARCH_CRAWLER_PATTERNS = [
  "bingbot",
  "adidxbot",
  "googlebot",
  "google-inspectiontool",
  "apis-google",
  "applebot",
  "duckduckbot",
  "slurp",
  "yandex",
  "baiduspider",
  "facebookexternalhit",
  "linkedinbot",
  "slackbot",
];

const DISCOVERY_LINKS = [
  `<${BASE}/llms.txt>; rel="alternate"; type="text/plain"`,
  `<${BASE}/api/>; rel="service-doc"; type="text/html"`,
];

export function wantsMarkdown(request) {
  const accept = request.headers.get("accept")?.toLowerCase() ?? "";
  const userAgent = request.headers.get("user-agent")?.toLowerCase() ?? "";
  if (SEARCH_CRAWLER_PATTERNS.some((pattern) => userAgent.includes(pattern))) return false;
  if (accept.includes("text/markdown")) return true;
  return AI_USER_AGENT_PATTERNS.some((pattern) => userAgent.includes(pattern));
}

/** `/docs/framework/` → `/docs/framework/index.md`; null for anything that is not a page URL. */
export function markdownPath(pathname) {
  if (pathname !== `${BASE}/` && !pathname.startsWith(`${BASE}/`)) return null;
  return pathname.endsWith("/") ? `${pathname}index.md` : null;
}

function withHeaders(response, extra) {
  const headers = new Headers(response.headers);
  // Link and Vary add to what the asset server sent; anything else replaces it.
  for (const [name, value] of extra) {
    if (name === "Link" || name === "Vary") headers.append(name, value);
    else headers.set(name, value);
  }
  return new Response(response.body, { status: response.status, statusText: response.statusText, headers });
}

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.hostname === OLD_HOST) {
      return Response.redirect(`${SITE}${BASE}${url.pathname}${url.search}`, 301);
    }

    // Preview deployments on workers.dev have nothing at the root.
    if (url.pathname === "/" || url.pathname === BASE) {
      return Response.redirect(new URL(`${BASE}/`, url), 301);
    }

    // The original announcement was shared publicly; keep that link current.
    const replacement = redirectedPagePath(url.pathname);
    if (replacement) {
      return Response.redirect(new URL(`${replacement}${url.search}`, url), 301);
    }

    const markdown = markdownPath(url.pathname);
    const vary = markdown ? [["Vary", "Accept, User-Agent"]] : [];
    const links = DISCOVERY_LINKS.map((link) => ["Link", link]);

    if (markdown && (request.method === "GET" || request.method === "HEAD") && wantsMarkdown(request)) {
      const twin = await env.ASSETS.fetch(new Request(new URL(markdown, url), request));
      if (twin.status === 200) {
        return withHeaders(twin, [
          ["Content-Type", "text/markdown; charset=utf-8"],
          ["X-Content-Negotiation", "markdown"],
          ...vary,
          ...links,
        ]);
      }
    }

    const response = await env.ASSETS.fetch(request);
    // auto-trailing-slash answers `/docs/page` with a 307 to `/docs/page/`.
    // That move is permanent, and a temporary redirect tells search engines to
    // keep the slashless URL; answer 301 instead, query kept.
    const location = response.headers.get("location");
    if (response.status === 307 && location) {
      const target = new URL(location, url);
      if (target.origin === url.origin && target.pathname === `${url.pathname}/`) {
        target.search = url.search;
        return Response.redirect(target.toString(), 301);
      }
    }
    return withHeaders(response, [...vary, ...(markdown ? links : [])]);
  },
};
