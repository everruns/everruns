import type { APIContext, GetStaticPaths } from "astro";
import { experimental_AstroContainer } from "astro/container";
import { getCollection, type CollectionEntry } from "astro:content";
import { htmlToSimpleMarkdown } from "starlight-llms-txt/entry-to-simple-markdown";
import { generatePageMarkdown } from "starlight-llms-txt/generator";
import { getSchemaStaticPaths, type StarlightOpenAPIRouteProps } from "starlight-openapi/route";
import OpenAPIPage from "starlight-openapi/routes/static";
import { PRODUCTS, isChangelogEntry } from "../src/lib/changelog";
import { changelogListMarkdown, withEntryFacts } from "../src/lib/changelog-markdown";

type Props =
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
  const docs = await getCollection("docs", (doc) => !doc.data.draft);
  const docsPaths = docs.map((doc) => {
    const path = pagePath(doc.id);
    return {
      params: { slug: path ? `${path}/index` : "index" },
      props: { kind: "docs" as const, doc, pageUrl: path ? `/${path}/` : "/" },
    };
  });
  const openAPIPaths = getSchemaStaticPaths().map(({ params, props }) => ({
    params: { slug: `${params.openAPISlug}/index` },
    props: {
      kind: "openapi" as const,
      pageUrl: `/${params.openAPISlug}/`,
      routeProps: props,
    },
  }));

  // Per-product changelog pages are Astro pages, not docs entries.
  const changelogPaths = PRODUCTS.map((p) => ({
    params: { slug: `changelog/${p.id}/index` },
    props: { kind: "changelog" as const, product: p.id, pageUrl: `/changelog/${p.id}/` },
  }));

  return [...docsPaths, ...openAPIPaths, ...changelogPaths];
}) satisfies GetStaticPaths;

export async function GET(context: APIContext<Props>) {
  const url = new URL(context.props.pageUrl, context.site);
  const pageContext = Object.create(context, {
    request: { value: new Request(url, context.request) },
    url: { value: url },
  }) as APIContext;
  const props = context.props;
  let body: string;
  if (props.kind === "changelog") {
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
  const content = await htmlToSimpleMarkdown(html, context, ".sl-markdown-content");
  const title =
    props.routeProps.type === "operation" ? props.routeProps.operation.title : "Overview";

  return [`# ${title}`, `Source: <${new URL(props.pageUrl, context.site).href}>`, content].join(
    "\n\n"
  );
}
