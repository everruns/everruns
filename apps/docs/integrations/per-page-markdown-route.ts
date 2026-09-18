import type { APIContext, GetStaticPaths } from "astro";
import { getCollection, getEntry } from "astro:content";
import { entryToSimpleMarkdown } from "starlight-llms-txt/entry-to-simple-markdown";

export const prerender = true;

export const getStaticPaths: GetStaticPaths = async () => {
  const docs = await getCollection("docs", ({ data }) => !data.draft);
  return docs
    .filter(({ id }) => id !== "" && id !== "index")
    .map(({ id }) => ({ params: { path: id } }));
};

export async function GET(context: APIContext): Promise<Response> {
  const id = context.params.path || "index";
  const doc = await getEntry("docs", id);
  if (!doc || doc.data.draft) {
    return new Response(null, { status: 404 });
  }

  const sourceUrl = new URL(id === "index" ? "./" : `${id}/`, context.site);
  const renderContext = Object.create(context, {
    request: { value: new Request(sourceUrl) },
    url: { value: sourceUrl },
  }) as APIContext;
  const segments = [`# ${doc.data.hero?.title || doc.data.title}`];
  const description = doc.data.hero?.tagline || doc.data.description;
  if (description) segments.push(`> ${description}`);
  segments.push(`Source: <${sourceUrl.href}>`);
  segments.push(await entryToSimpleMarkdown(doc, renderContext));

  return new Response(`${segments.join("\n\n")}\n`, {
    headers: { "content-type": "text/markdown; charset=utf-8" },
  });
}
