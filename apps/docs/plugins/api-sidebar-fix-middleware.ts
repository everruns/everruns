// Post-order middleware that fixes sidebar-topics for API pages.
// Runs after starlight-openapi has populated the sidebar with real items.

import { defineRouteMiddleware } from "@astrojs/starlight/route-data";

// The Reference topic index in the sidebar-topics config array.
const REFERENCE_TOPIC_INDEX = 4;

export const onRequest = defineRouteMiddleware((context) => {
  const { starlightRoute } = context.locals;
  const slug = starlightRoute.entry?.id;
  if (!slug?.startsWith("api/") && slug !== "api") return;

  // Only fix pages that sidebar-topics excluded (isPageWithTopic === false).
  const topicsData = context.locals.starlightSidebarTopics;
  if (!topicsData || topicsData.isPageWithTopic) return;

  // Find the Reference topic's sidebar group (numbered by index).
  const referenceGroup = starlightRoute.sidebar.find(
    (item) =>
      item.type === "group" && item.label === String(REFERENCE_TOPIC_INDEX),
  );

  if (referenceGroup?.type === "group") {
    // Replace the full sidebar with just the Reference topic's items.
    starlightRoute.sidebar = referenceGroup.entries;
  }

  // Patch sidebar-topics locals so the topic tabs render correctly.
  topicsData.isPageWithTopic = true;
  for (const topic of topicsData.topics) {
    topic.isCurrent = topic.link === "/api/";
  }
});
