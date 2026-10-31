// Renderer metadata only. Agent runs and actions stay on the existing APIs.
export function GET() {
  return Response.json(
    { version: "1.83.0", agents: {}, telemetryDisabled: true },
    {
      headers: { "Cache-Control": "public, max-age=86400" },
    },
  );
}
