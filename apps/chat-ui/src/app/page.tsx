import { headers } from "next/headers";
import { notFound } from "next/navigation";
import { channelForHost, bootstrap } from "../server/config";
import { Chat } from "../components/chat";
export const dynamic = "force-dynamic";
export default async function Page() {
  const host = (await headers()).get("host") ?? "";
  let channel: string;
  try {
    channel = await channelForHost(new URL(`http://${host}`).hostname);
  } catch {
    return notFound();
  }
  let config;
  try {
    config = await bootstrap(channel);
  } catch {
    return notFound();
  }
  return <Chat config={config} />;
}
