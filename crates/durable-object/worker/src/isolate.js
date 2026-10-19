import { HttpError, requireValue } from "./storage.js";

// The bridge is object-local and bound to the activation's session. It never
// accepts an object address or callback URL from agent-visible input.
export function objectPort(authority, scope) {
  return async (encoded) => {
    try {
      requireValue(
        typeof encoded === "string" && new TextEncoder().encode(encoded).length <= 1024 * 1024,
        413,
        "bridge request exceeds limit",
      );
      const request = JSON.parse(encoded);
      requireValue(
        request.tenant === scope.tenant && request.session === scope.session_id,
        404,
        "bridge scope mismatch",
      );
      const value = await authority.handle(
        request.operation,
        request.body,
        request.session,
        request.tenant,
      );
      return JSON.stringify({ status: 200, value });
    } catch (error) {
      return JSON.stringify({
        status: error instanceof HttpError ? error.status : 503,
        value: null,
      });
    }
  };
}
