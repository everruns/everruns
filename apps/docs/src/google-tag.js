// Google tag (gtag.js), the same Google Ads account as everruns.com
// (everruns/landing src/components/GoogleTag.astro), so a docs visit that ends
// in a signup or a GitHub visit is attributed like one on the landing site.
// Inlined into every page's <head> by astro.config.mjs. The `subscribe` event
// is fired by the shared footer form (src/site-chrome/chrome.js).
window.dataLayer = window.dataLayer || [];
function gtag() {
  dataLayer.push(arguments);
}
window.gtag = gtag;
gtag("js", new Date());
gtag("config", "AW-18025687982");

// Outbound CTA tracking, same event names as the landing site.
document.addEventListener(
  "click",
  (event) => {
    const link = event.target.closest?.("a[href]");
    if (!link) return;
    let host;
    try {
      host = new URL(link.href, window.location.origin).hostname;
    } catch {
      return;
    }
    const name =
      host === "app.everruns.com"
        ? "cloud_signup_click"
        : host === "github.com" && link.href.includes("/everruns")
          ? "github_click"
          : null;
    if (!name) return;
    gtag("event", name, {
      link_text: (link.textContent || "").trim().slice(0, 100),
      link_url: link.href,
      page_path: window.location.pathname,
    });
  },
  { capture: true },
);
