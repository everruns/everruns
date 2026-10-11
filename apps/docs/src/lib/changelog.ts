// Shared taxonomy and helpers for the public changelog (docs/changelog/).
//
// Decision: changelog entries are ordinary pages in the Starlight `docs`
// collection, one folder per entry (`changelog/<date>-<slug>/index.md` with its
// screenshots and videos beside it). That gives them search, per-page Markdown,
// llms.txt, link validation and Open Graph cards with no parallel pipeline. This
// module only adds what a list of dated entries needs on top: the taxonomy,
// sorting, and the availability line. Design: knowledge/docs/changelog.md.
import { getCollection, type CollectionEntry } from "astro:content";

export const PRODUCTS = [
  { id: "cloud", label: "Everruns Cloud" },
  { id: "platform", label: "Self-hosted" },
  { id: "framework", label: "Framework" },
  { id: "sdk", label: "SDKs" },
  { id: "cli", label: "CLI" },
  { id: "yolop", label: "Yolop" },
  { id: "mira", label: "Mira" },
  { id: "bashkit", label: "Bashkit" },
] as const;

export const SURFACES = [
  { id: "ui", label: "UI" },
  { id: "api", label: "API" },
  { id: "code", label: "Code" },
  { id: "cli", label: "CLI" },
  { id: "agent", label: "Agent protocols" },
  { id: "operations", label: "Operations" },
] as const;

export const KINDS = [
  { id: "feature", label: "Feature" },
  { id: "improvement", label: "Improvement" },
  { id: "fix", label: "Fix" },
  { id: "breaking", label: "Breaking" },
  { id: "deprecation", label: "Deprecation" },
  { id: "incident", label: "Incident" },
  { id: "release", label: "Release" },
] as const;

export const SIGNIFICANCE = [
  { id: "major", label: "Major" },
  { id: "notable", label: "Notable" },
  { id: "minor", label: "Minor" },
] as const;

export type ProductId = (typeof PRODUCTS)[number]["id"];
export type Entry = CollectionEntry<"docs">;

const labelOf = (list: readonly { id: string; label: string }[], id: string) =>
  list.find((item) => item.id === id)?.label ?? id;

export const productLabel = (id: string) => labelOf(PRODUCTS, id);
export const surfaceLabel = (id: string) => labelOf(SURFACES, id);
export const kindLabel = (id: string) => labelOf(KINDS, id);
export const significanceLabel = (id: string) => labelOf(SIGNIFICANCE, id);

export const isChangelogEntry = (doc: Entry) =>
  doc.id.startsWith("changelog/") && doc.data.date !== undefined;

export const entryUrl = (doc: Entry) => `/${doc.id.replace(/\/index$/, "")}/`;

/** Every entry, newest first; ties keep a stable order by id. */
export async function getChangelogEntries(): Promise<Entry[]> {
  const docs = await getCollection("docs", (doc) => !doc.data.draft && isChangelogEntry(doc));
  return docs.sort(
    (a, b) => b.data.date!.getTime() - a.data.date!.getTime() || b.id.localeCompare(a.id),
  );
}

export const formatDate = (date: Date) =>
  date.toLocaleDateString("en-US", { year: "numeric", month: "long", day: "numeric", timeZone: "UTC" });

export const isoDate = (date: Date) => date.toISOString().slice(0, 10);

// The product a `release` version belongs to. Platform, Framework and CLI share
// the everruns version; the other products version on their own.
const RELEASE_REPO: Record<string, string> = {
  yolop: "everruns/yolop",
  sdk: "everruns/sdk",
  mira: "everruns/mira",
  bashkit: "everruns/bashkit",
};

/** Where a reader can use the change today, in the order a feature reaches them. */
export function availability(doc: Entry): { label: string; href?: string }[] {
  const { products = [], release, cloud, kind } = doc.data;
  if (kind === "release" || kind === "incident") return [];
  const out: { label: string; href?: string }[] = [];
  const own = products.find((p: string) => p in RELEASE_REPO);
  if (release) {
    const repo = own && !products.some((p: string) => ["platform", "framework", "cli"].includes(p))
      ? RELEASE_REPO[own]
      : "everruns/everruns";
    const name = repo === "everruns/everruns" ? "" : `${productLabel(own!)} `;
    out.push({
      label: `In ${name}${release}`,
      href: `https://github.com/${repo}/releases/tag/v${release}`,
    });
  } else if (products.some((p: string) => p !== "cloud")) {
    out.push({ label: "On main, not yet released" });
  }
  if (cloud) {
    out.push({ label: `Live on Cloud since ${formatDate(cloud)}`, href: "https://app.everruns.com" });
  }
  return out;
}

/** `everruns/everruns#123` → GitHub URL and short label. */
export function prLink(ref: string): { href: string; label: string } {
  const match = ref.match(/^([\w.-]+\/[\w.-]+)#(\d+)$/);
  if (!match) return { href: ref, label: ref };
  const [, repo, number] = match;
  const short = repo === "everruns/everruns" ? `#${number}` : `${repo.split("/")[1]}#${number}`;
  return { href: `https://github.com/${repo}/pull/${number}`, label: short };
}
