// Caddy's certificate admission endpoint. A wildcard DNS record alone must
// never let an arbitrary host trigger certificate issuance or reach the API.
import { bootstrap, channelForHost, settings } from "../../../server/config";
export async function GET(request: Request) {
  const domain = new URL(request.url).searchParams.get("domain") ?? "";
  try {
    if (domain === `login.${settings().domain}`) return new Response(null, { status: 200 });
    await bootstrap(await channelForHost(domain));
    return new Response(null, { status: 200 });
  } catch {
    return new Response(null, { status: 404 });
  }
}
