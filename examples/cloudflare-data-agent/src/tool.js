// The one tool this agent exposes. The model chooses the region; the
// SQL is fixed and every parameter is bound, so the model never writes
// a query. The database stays in your Cloudflare account: only the tool
// result is reported to Everruns.

export const SALES_SUMMARY_TOOL = {
  type: "function",
  name: "sales_summary",
  description:
    "Read total sales and order count for a region " +
    "from the private sales database.",
  parameters: {
    type: "object",
    properties: { region: { type: "string" } },
    required: ["region"],
    additionalProperties: false,
  },
};

const SALES_SUMMARY_SQL = `
  SELECT COUNT(*) AS order_count,
         COALESCE(SUM(amount_cents), 0) AS total_cents
  FROM orders
  WHERE region = ?`;

export async function salesSummary(database, args) {
  if (typeof args.region !== "string" || args.region.length > 128) {
    throw new Error("Invalid region");
  }
  return await database
    .prepare(SALES_SUMMARY_SQL)
    .bind(args.region)
    .first();
}
