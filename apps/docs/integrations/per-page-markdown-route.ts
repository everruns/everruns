import type { APIContext, GetStaticPaths } from "astro";
import { experimental_AstroContainer } from "astro/container";
import { getCollection, type CollectionEntry } from "astro:content";
import { htmlToSimpleMarkdown } from "starlight-llms-txt/entry-to-simple-markdown";
import { generatePageMarkdown } from "starlight-llms-txt/generator";
import { getSchemaStaticPaths, type StarlightOpenAPIRouteProps } from "starlight-openapi/route";
import OpenAPIPage from "starlight-openapi/routes/static";
import ApiGroupPage from "../src/components/ApiGroupPage.astro";
import { allGroups, operationTitle, byId, type ApiGroup } from "../src/lib/api-reference";
import { PRODUCTS, isChangelogEntry } from "../src/lib/changelog";
import { changelogListMarkdown, withEntryFacts } from "../src/lib/changelog-markdown";
import { withBase } from "../site.config.mjs";

type Props =
  | { kind: "api-group"; group: ApiGroup; pageUrl: string }
  | {
      kind: "docs";
      doc: CollectionEntry<"docs">;
      pageUrl: string;
    }
  | {
      kind: "changelog";
      product: string;
      pageUrl: string;
    }
  | {
      kind: "openapi";
      pageUrl: string;
      routeProps: StarlightOpenAPIRouteProps;
    };

const pagePath = (id: string) => (id === "" || id === "index" ? "" : id.replace(/\/+$/, ""));
const astroContainer = await experimental_AstroContainer.create();

export const getStaticPaths = (async () => {
  // The 404 page (docs/404.mdx) is site chrome with no HTML route of its own.
  const docs = await getCollection("docs", (doc) => !doc.data.draft && doc.id !== "404");
  const docsPaths = docs.map((doc) => {
    const path = pagePath(doc.id);
    return {
      params: { slug: path ? `${path}/index` : "index" },
      props: { kind: "docs" as const, doc, pageUrl: withBase(path ? `/${path}/` : "/") },
    };
  });
  const openAPIPaths = getSchemaStaticPaths().map(({ params, props }) => ({
    params: { slug: `${params.openAPISlug}/index` },
    props: {
      kind: "openapi" as const,
      pageUrl: withBase(`/${params.openAPISlug}/`),
      routeProps: props,
    },
  }));

  // Per-product changelog pages are Astro pages, not docs entries.
  const changelogPaths = PRODUCTS.map((p) => ({
    params: { slug: `changelog/${p.id}/index` },
    props: { kind: "changelog" as const, product: p.id, pageUrl: withBase(`/changelog/${p.id}/`) },
  }));

  const groupPaths = allGroups.map(group => ({
    params: { slug: `api/groups/${group.id}/index` },
    props: { kind: "api-group" as const, group, pageUrl: withBase(`/api/groups/${group.id}/`) },
  }));
  return [...docsPaths, ...openAPIPaths, ...changelogPaths, ...groupPaths];
}) satisfies GetStaticPaths;

export async function GET(context: APIContext<Props>) {
  const url = new URL(context.props.pageUrl, context.site);
  const pageContext = Object.create(context, {
    request: { value: new Request(url, context.request) },
    url: { value: url },
  }) as APIContext;
  const props = context.props;
  let body: string;
  if (props.kind === "api-group") {
    const html = await astroContainer.renderToString(ApiGroupPage, { locals: context.locals, props: { group: props.group }, request: context.request });
    body = `Source: <${url.href}>\n\n${await htmlToSimpleMarkdown(html, pageContext, ".api-main")}`;
  } else if (props.kind === "changelog") {
    body = await changelogListMarkdown(props.product);
  } else if (props.kind === "docs" && props.doc.id === "changelog") {
    body = await changelogListMarkdown();
  } else if (props.kind === "docs") {
    body = await generatePageMarkdown(props.doc, pageContext);
    if (isChangelogEntry(props.doc)) body = withEntryFacts(body, props.doc);
  } else {
    body = await generateOpenAPIMarkdown(props, pageContext);
  }

  return new Response(body, {
    headers: { "Content-Type": "text/markdown; charset=utf-8" },
  });
}

async function generateOpenAPIMarkdown(
  props: Extract<Props, { kind: "openapi" }>,
  context: APIContext
) {
  const html = await astroContainer.renderToString(OpenAPIPage, {
    locals: context.locals,
    props: props.routeProps as unknown as Record<string, unknown>,
    request: context.request,
  });
  const content = await htmlToSimpleMarkdown(html, context, ".api-main");
  const title =
    props.routeProps.type === "operation" ? operationTitle(byId.get(props.routeProps.operation.operation.operationId!)!) : "API Reference";

  return [`# ${title}`, `Source: <${new URL(props.pageUrl, context.site).href}>`, content].join(
    "\n\n"
  );
}
