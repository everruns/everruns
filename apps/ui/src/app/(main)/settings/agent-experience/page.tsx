import { redirect } from "next/navigation";

// The combined page is now Profile, Connections, MCP servers, and AI clients.
// An OAuth result still targets this URL from older return paths; Connections
// is the page that reads `connected` and `connect_error`.
export default async function AgentExperiencePage({
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
  const oauthReturn = ["connected", "connect_error", "provider"].some((key) => query.has(key));
  const path = oauthReturn ? "/settings/connections" : "/settings/user-profile";
  redirect(`${path}${encoded ? `?${encoded}` : ""}`);
}
