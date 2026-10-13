import { operations, operationTitle } from "../../lib/api-reference";

export function GET() {
  return new Response(JSON.stringify(operations.map(op => ({
    title: operationTitle(op), path: op.path, id: op.id, method: op.method, href: op.href,
    support: op.docs.support, deprecated: op.deprecated, audience: op.docs.audience,
  }))), { headers: { "Content-Type": "application/json; charset=utf-8" } });
}
