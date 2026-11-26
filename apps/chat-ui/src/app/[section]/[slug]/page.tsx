import { headers } from "next/headers";
import { notFound } from "next/navigation";
import { bootstrap, channelForHost } from "../../../server/config";
import { Application } from "../../../components/application";
export const dynamic = "force-dynamic";
export default async function Page({
  params,
}: {
  params: Promise<{ section: string; slug: string }>;
}) {
  const { section, slug } = await params;
  if (
    !["chats", "rooms", "memory"].includes(section) ||
    !/^[a-z][a-z0-9-]{1,46}[a-z0-9]$/.test(slug)
  )
    return notFound();
  try {
    const host = new URL(`https://${(await headers()).get("host")}`).hostname;
    const target = await channelForHost(host);
    if (!target.startsWith("application_")) return notFound();
    return <Application config={await bootstrap(target)} />;
  } catch {
    return notFound();
  }
}
