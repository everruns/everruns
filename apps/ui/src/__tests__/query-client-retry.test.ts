/**
 * Queries retry only failures a second attempt can fix.
 *
 * A 404 retried before the cross-org resolver could switch doubled every miss
 * of a link opened in the wrong org, and a retried 429 answered the edge rate
 * limit with more requests.
 */
import { ApiError } from "@/lib/api/client";
import { shouldRetryQuery } from "@/lib/query-client";

test("4xx answers are final", () => {
  for (const status of [400, 401, 403, 404, 409, 429]) {
    expect(shouldRetryQuery(0, new ApiError(status, "x"))).toBe(false);
  }
});

test("server errors and network failures retry once", () => {
  expect(shouldRetryQuery(0, new ApiError(503, "Service Unavailable"))).toBe(true);
  expect(shouldRetryQuery(1, new ApiError(503, "Service Unavailable"))).toBe(false);
  expect(shouldRetryQuery(0, new TypeError("Failed to fetch"))).toBe(true);
  expect(shouldRetryQuery(1, new TypeError("Failed to fetch"))).toBe(false);
});
