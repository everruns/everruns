// Route middleware for SEO improvements on auto-generated pages.
//
// 1. Generates fallback meta descriptions for pages without explicit ones
//    (primarily API reference pages from starlight-openapi).
// 2. Strips "METHOD /path - " prefix from API operation page titles to keep
//    <title> tags under 70 chars. The method and path are already shown
//    separately on the page by the starlight-openapi Operation component.
//
// Head entries are already computed before middleware runs, so we modify
// route.head directly (not just route.entry.data).

import { defineRouteMiddleware } from "@astrojs/starlight/route-data";

// Matches "GET /v1/foo - Description" or "POST /v1/foo/{id}/bar - Description"
const methodPathPrefix =
  /^(?:GET|POST|PUT|PATCH|DELETE|HEAD|OPTIONS)\s+\S+\s+-\s+/;

export const onRequest = defineRouteMiddleware((context) => {
  const route = context.locals.starlightRoute;
  if (!route.entry?.id) return;

  const slug = route.entry.id;
  const isApiOperation =
    slug.startsWith("api/operations/") &&
    !slug.startsWith("api/operations/tags/");

  // Strip method+path prefix from API operation titles for shorter <title> tags.
  // Update both entry.data.title (affects h1) and the <title> head entry.
  if (isApiOperation) {
    const cleanTitle = route.entry.data.title.replace(methodPathPrefix, "");
    route.entry.data.title = cleanTitle;

    // Update the <title> head entry
    const titleEntry = route.head.find((e) => e.tag === "title");
    if (titleEntry) {
      titleEntry.content = titleEntry.content?.replace(methodPathPrefix, "");
    }

    // Update og:title
    const ogTitle = route.head.find(
      (e) =>
        e.tag === "meta" &&
        e.attrs &&
        "property" in e.attrs &&
        e.attrs.property === "og:title",
    );
    if (ogTitle?.attrs) {
      ogTitle.attrs.content = cleanTitle;
    }
  }

  // Generate meta description for pages without one.
  // Check if a page-specific description already exists in the head.
  const hasDescription = route.head.some(
    (e) =>
      e.tag === "meta" &&
      e.attrs &&
      "name" in e.attrs &&
      e.attrs.name === "description",
  );

  if (!hasDescription || !route.entry.data.description) {
    const description = deriveDescription(slug, route.entry.data.title);
    route.entry.data.description = description;

    // Replace or add meta description in head
    const descIdx = route.head.findIndex(
      (e) =>
        e.tag === "meta" &&
        e.attrs &&
        "name" in e.attrs &&
        e.attrs.name === "description",
    );
    const descEntry = {
      tag: "meta" as const,
      attrs: { name: "description", content: description },
      content: "",
    };
    if (descIdx >= 0) {
      route.head[descIdx] = descEntry;
    } else {
      route.head.push(descEntry);
    }

    // Also update/add og:description
    const ogDescIdx = route.head.findIndex(
      (e) =>
        e.tag === "meta" &&
        e.attrs &&
        "property" in e.attrs &&
        e.attrs.property === "og:description",
    );
    const ogDescEntry = {
      tag: "meta" as const,
      attrs: { property: "og:description", content: description },
      content: "",
    };
    if (ogDescIdx >= 0) {
      route.head[ogDescIdx] = ogDescEntry;
    } else {
      route.head.push(ogDescEntry);
    }
  }
});

function deriveDescription(slug: string, title: string): string {
  // API tag group pages: derive tag name from slug (e.g. "api/operations/tags/agents" → "agents")
  if (slug.startsWith("api/operations/tags/")) {
    const tagName = slug.split("/").pop() ?? title;
    return `Browse all Everruns API ${tagName} endpoints, including request parameters, response schemas, and usage examples for each operation.`;
  }

  // API operation pages (title is already cleaned of method+path prefix)
  if (slug.startsWith("api/operations/")) {
    return `${title} — Everruns REST API reference. View request parameters, response schema, and example payloads for this endpoint.`;
  }

  // API overview page
  if (slug === "api") {
    return "Complete API reference for Everruns, including endpoints, schemas, and authentication";
  }

  // API schema pages
  if (slug.startsWith("api/schemas/")) {
    return `${title} schema definition in the Everruns REST API. View fields, types, required properties, and relationships for this data model.`;
  }

  // Generic fallback
  return `${title} — Everruns documentation. Learn how to deploy, configure, and build AI agent applications with the durable harness engine.`;
}
