// Google tag (gtag.js), the same Google Ads account as everruns.com
// (everruns/landing src/components/GoogleTag.astro). Inlined into every page's
// <head> by astro.config.mjs.
//
// Decision (owner, 2026-11-12): page views only. The default `config` call
// records the page view; do not add click, form or conversion events.
window.dataLayer = window.dataLayer || [];
function gtag() {
  dataLayer.push(arguments);
}
gtag("js", new Date());
gtag("config", "AW-18025687982");
