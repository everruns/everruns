// Components build docs links with these helpers, never a hard-coded "/..."
// path, because Astro does not add the base to hand-written URLs.
export { docsUrl, withBase as docsPath } from "../../site.config.mjs";
