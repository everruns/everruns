/// Full-page navigation to an external URL. Isolated so tests can mock it
/// (jsdom cannot redefine `window.location`).
export function navigateTo(url: string) {
  window.location.href = url;
}
