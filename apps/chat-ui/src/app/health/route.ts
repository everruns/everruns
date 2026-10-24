import { ready } from "../../server/store";
export async function GET() {
  try {
    await ready();
    return Response.json({ status: "ok" });
  } catch {
    return Response.json({ status: "unavailable" }, { status: 503 });
  }
}
