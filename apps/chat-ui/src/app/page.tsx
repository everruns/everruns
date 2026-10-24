import { headers } from "next/headers";
import { notFound } from "next/navigation";
import { settings, channelFromHost, bootstrap } from "../server/config";
import { Chat } from "../components/chat";
export const dynamic = "force-dynamic";
export default async function Page() {
  const host = (await headers()).get("host") ?? "";
  const channel = channelFromHost(new URL(`http://${host}`).hostname, settings().domain);
  if (!channel) notFound();
  let config;
  try {
    config = await bootstrap(channel);
  } catch {
    notFound();
  }
  return <Chat config={config} />;
}
