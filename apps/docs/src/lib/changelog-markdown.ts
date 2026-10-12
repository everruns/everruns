// Markdown views of the changelog for coding agents (`/changelog/index.md`,
// `/changelog/<product>/index.md`, and the header of each entry's `index.md`).
//
// The HTML list carries filter controls and tag chips that convert to noise,
// so the list pages are written directly as Markdown: one line per entry with
// its date, products, kind and release. Entry pages keep their converted body
// and gain the same facts as a short header, so an agent can tell whether a
// change is in the version it runs without opening the HTML.
import { docsUrl } from "./site";
import {
  availability,
  entryUrl,
  getChangelogEntries,
  isoDate,
  kindLabel,
  productLabel,
  productName,
  type Entry,
} from "./changelog";


const facts = (entry: Entry) => {
  const d = entry.data;
  return [
    isoDate(d.date!),
    (d.products ?? []).map(productLabel).join(", "),
    d.kind ? kindLabel(d.kind) : "",
    d.significance === "major" ? "Major" : "",
    ...availability(entry).map((a) => a.label),
  ].filter(Boolean);
};

export async function changelogListMarkdown(product?: string): Promise<string> {
  const entries = (await getChangelogEntries()).filter(
    (e) => !product || (e.data.products as string[] | undefined)?.includes(product),
  );
  const page = docsUrl(product ? `/changelog/${product}/` : "/changelog/");
  const title = product ? `${productName(product)} changelog` : "Changelog";
  const lines = [
    `# ${title}`,
    "",
    "> One entry per finished change, newest first. Each line: date, products, kind, and where the change is available.",
    "",
    `Source: <${page}>`,
    "",
    `Feed: <${page}rss.xml>`,
    "",
  ];
  for (const entry of entries) {
    const superseded = entry.data.supersededBy ? " (superseded)" : "";
    lines.push(
      `- [${entry.data.title}](${docsUrl(entryUrl(entry))})${superseded}: ${facts(entry).join(" · ")}. ${entry.data.description ?? ""}`,
    );
  }
  return `${lines.join("\n")}\n`;
}

/** Inserts the entry's facts under its Source line. */
export function withEntryFacts(markdown: string, entry: Entry): string {
  const d = entry.data;
  const header = [
    `Date: ${isoDate(d.date!)}`,
    `Products: ${(d.products ?? []).map(productLabel).join(", ")}`,
    d.kind ? `Category: ${kindLabel(d.kind)}` : "",
    availability(entry).length ? `Available: ${availability(entry).map((a) => a.label).join("; ")}` : "",
    d.supersededBy ? `Superseded by: ${docsUrl(`/changelog/${d.supersededBy}/`)}` : "",
    ...(d.links?.prs ?? []).map((pr) => `Pull request: ${pr}`),
  ].filter(Boolean);
  return markdown.replace(/(Source: <[^>]+>\n)/, `$1\n${header.join("\n")}\n`);
}
