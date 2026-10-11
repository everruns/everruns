// RSS 2.0 for the changelog: one feed for every entry, one per product.
//
// Items carry the full rendered entry, not just the description, so a reader
// never has to leave the feed to see what changed. Relative links and media in
// an entry resolve against the entry's own URL (its media sit in its folder).
import { experimental_AstroContainer } from "astro/container";
import { render } from "astro:content";
import { entryUrl, getChangelogEntries, kindLabel, productLabel, productName, type Entry } from "./changelog";

const SITE = "https://docs.everruns.com";
const container = await experimental_AstroContainer.create();

const xml = (s: string) =>
  s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");

function absolutize(html: string, base: string): string {
  return html.replace(/\b(href|src|poster)="(?!https?:|mailto:|#|data:)([^"]*)"/g, (_, attr, url) => {
    return `${attr}="${new URL(url, base).href}"`;
  });
}

async function item(entry: Entry): Promise<string> {
  const url = `${SITE}${entryUrl(entry)}`;
  const { Content } = await render(entry);
  const html = absolutize(await container.renderToString(Content), url);
  const categories = [
    ...(entry.data.products ?? []).map(productLabel),
    ...(entry.data.kind ? [kindLabel(entry.data.kind)] : []),
  ];
  return [
    "<item>",
    `<title>${xml(entry.data.title)}</title>`,
    `<link>${url}</link>`,
    `<guid isPermaLink="true">${url}</guid>`,
    `<pubDate>${entry.data.date!.toUTCString()}</pubDate>`,
    `<description>${xml(entry.data.description ?? "")}</description>`,
    `<content:encoded><![CDATA[${html.replaceAll("]]>", "]]]]><![CDATA[>")}]]></content:encoded>`,
    ...categories.map((c) => `<category>${xml(c)}</category>`),
    "</item>",
  ].join("");
}

export async function changelogFeed(product?: string): Promise<Response> {
  const entries = (await getChangelogEntries()).filter(
    (e) => !product || (e.data.products as string[] | undefined)?.includes(product),
  );
  const title = product ? `${productName(product)} changelog` : "Everruns changelog";
  const page = product ? `${SITE}/changelog/${product}/` : `${SITE}/changelog/`;
  const self = product ? `${SITE}/changelog/${product}/rss.xml` : `${SITE}/changelog/rss.xml`;
  const items = await Promise.all(entries.map(item));
  const body = [
    '<?xml version="1.0" encoding="UTF-8"?>',
    '<rss version="2.0" xmlns:atom="http://www.w3.org/2005/Atom" xmlns:content="http://purl.org/rss/1.0/modules/content/">',
    "<channel>",
    `<title>${xml(title)}</title>`,
    `<link>${page}</link>`,
    `<atom:link href="${self}" rel="self" type="application/rss+xml"/>`,
    `<description>${xml(`What changed in ${product ? productName(product) : "Everruns"}, one short entry per change.`)}</description>`,
    "<language>en</language>",
    entries[0] ? `<lastBuildDate>${entries[0].data.date!.toUTCString()}</lastBuildDate>` : "",
    ...items,
    "</channel>",
    "</rss>",
  ].join("\n");
  return new Response(body, { headers: { "Content-Type": "application/rss+xml; charset=utf-8" } });
}
