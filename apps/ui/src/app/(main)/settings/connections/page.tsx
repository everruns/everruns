import { redirect } from "next/navigation";

// The sidebar lists connections once, on My agent experience. This URL stays
// so setup links and OAuth returns keep landing on that page.
export default async function ConnectionsPage({
  searchParams,
}: {
  searchParams: Promise<Record<string, string | string[] | undefined>>;
}) {
  const params = await searchParams;
  const query = new URLSearchParams();
  for (const [key, value] of Object.entries(params)) {
    if (typeof value === "string") {
      query.append(key, value);
    } else if (Array.isArray(value)) {
      for (const item of value) query.append(key, item);
    }
  }
  const encoded = query.toString();
  redirect(`/settings/agent-experience${encoded ? `?${encoded}` : ""}`);
}
