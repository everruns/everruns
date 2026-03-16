// Fix sidebar-topics + starlight-openapi middleware ordering for API pages.
//
// Problem: starlight-sidebar-topics (pre) runs before starlight-openapi (post).
// When sidebar-topics processes API pages, the sidebar still has placeholder
// items so it can't match them to the Reference topic. The workaround was to
// exclude /api/** from topic matching, but that caused the sidebar to render
// raw numbered groups instead of the Reference topic's sidebar.
//
// Solution: This plugin registers a post-order middleware that runs AFTER
// starlight-openapi populates the sidebar. For API pages that sidebar-topics
// excluded, it extracts the Reference topic's sidebar group and patches the
// sidebar-topics locals so the page is treated as belonging to the Reference
// topic.

import type { StarlightPlugin } from "@astrojs/starlight/types";

// The Reference topic is at this index in the topics config array.
// Must match the position in astro.config.mjs starlightSidebarTopics() call.
const REFERENCE_TOPIC_INDEX = 4;

export default function apiSidebarFix(): StarlightPlugin {
  return {
    name: "api-sidebar-fix",
    hooks: {
      "config:setup"({ addRouteMiddleware }) {
        addRouteMiddleware({
          entrypoint: new URL("./api-sidebar-fix-middleware.ts", import.meta.url)
            .pathname,
          order: "post",
        });
      },
    },
  };
}
