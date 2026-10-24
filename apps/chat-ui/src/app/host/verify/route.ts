// Caddy's certificate admission endpoint. A wildcard DNS record alone must
// never let an arbitrary host trigger certificate issuance or reach the API.
import { bootstrap, channelFromHost, settings } from "../../../server/config";
export async function GET(request: Request) {
  const domain = new URL(request.url).searchParams.get("domain") ?? "";
  try {
    if (domain === `login.${settings().domain}`) return new Response(null, { status: 200 });
    const channel = channelFromHost(domain, settings().domain);
    if (!channel) return new Response(null, { status: 404 });
    await bootstrap(channel);
    return new Response(null, { status: 200 });
  } catch {
    return new Response(null, { status: 404 });
  }
}
