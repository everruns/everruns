import { ready } from "../../server/store";
import { settings } from "../../server/config";
export const dynamic = "force-dynamic";
export async function GET(request: Request) {
  try {
    const config = settings();
    if (!/^[a-f0-9]{64}$/i.test(config.key)) throw new Error("Invalid key");
    if (new URL(request.url).searchParams.get("ready") === "1") await ready();
    return Response.json({ status: "ok" });
  } catch {
    return Response.json({ status: "unavailable" }, { status: 503 });
  }
}
