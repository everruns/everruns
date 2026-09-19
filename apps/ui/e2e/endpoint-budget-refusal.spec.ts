import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { expect, test, type APIResponse, type Page } from "@playwright/test";

const apiBaseUrl = process.env.PLAYWRIGHT_REAL_API_URL;
const databaseUrl = process.env.DATABASE_URL;
test.use({ trace: "on" });

async function jsonResponse<T>(response: APIResponse): Promise<T> {
  if (!response.ok()) {
    throw new Error(`${response.status()} ${response.statusText()}: ${await response.text()}`);
  }
  return response.json() as Promise<T>;
}

async function proxyApiToRealServer(page: Page) {
  await page.route("**/api/**", async (route) => {
    const requestUrl = new URL(route.request().url());
    const upstream = new URL(`${requestUrl.pathname}${requestUrl.search}`, apiBaseUrl!);
    const response = await route.fetch({ url: upstream.toString() });
    await route.fulfill({ response });
  });
}

test.describe("endpoint budget refusal", () => {
  test.skip(
    !apiBaseUrl || !databaseUrl,
    "Requires PLAYWRIGHT_REAL_API_URL and DATABASE_URL for the real PostgreSQL-backed server.",
  );

  test("shows the cap whose stable ID appears in a real refusal", async ({
    page,
    request,
  }, testInfo) => {
    const agent = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/agents`, {
        data: {
          name: `budget-e2e-${randomUUID()}`,
          display_name: "Endpoint budget refusal",
          system_prompt: "Test endpoint budget traceability.",
        },
      }),
    );

    const apiKey = `evr_app_${randomUUID().replaceAll("-", "")}`;
    const endpoint = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/agents/${agent.id}/endpoints`, {
        data: {
          channel_type: "api_endpoint",
          channel_config: {
            session_mode: "session_per_invocation",
            api_key_hash: createHash("sha256").update(apiKey).digest("hex"),
            api_key_prefix: apiKey.slice(0, 12),
          },
          enabled: true,
        },
      }),
    );
    await jsonResponse(
      await request.post(
        `${apiBaseUrl}/api/v1/agents/${agent.id}/endpoints/${endpoint.id}/publish`,
      ),
    );

    const budget = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/budgets`, {
        data: {
          subject_type: "agent_endpoint",
          subject_id: endpoint.id,
          currency: "tokens",
          limit: 25,
          period: { type: "duration", seconds: 3600 },
        },
      }),
    );

    const session = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/sessions`, {
        data: {
          agent_id: agent.id,
          title: "Endpoint budget refusal",
        },
      }),
    );

    const attributed = execFileSync(
      "psql",
      [
        databaseUrl!,
        "--no-psqlrc",
        "--set",
        "ON_ERROR_STOP=1",
        "--set",
        `session_id=${session.id}`,
        "--set",
        `endpoint_id=${endpoint.id}`,
        "--command",
        "UPDATE sessions AS session SET endpoint_id = endpoint.id FROM agent_endpoints AS endpoint WHERE endpoint.public_id = :'endpoint_id' AND replace(session.id::text, '-', '') = substring(:'session_id' from 9);",
      ],
      { encoding: "utf8" },
    );
    expect(attributed).toContain("UPDATE 1");
    const exhausted = execFileSync(
      "psql",
      [
        databaseUrl!,
        "--no-psqlrc",
        "--set",
        "ON_ERROR_STOP=1",
        "--set",
        `endpoint_id=${endpoint.id}`,
        "--command",
        "UPDATE budgets SET balance = 0, status = 'exhausted', period_started_at = NOW() WHERE subject_type = 'agent_endpoint' AND subject_id = :'endpoint_id';",
      ],
      { encoding: "utf8" },
    );
    expect(exhausted).toContain("UPDATE 1");

    const refusal = await jsonResponse<{
      action: string;
      budget_id: string;
      error_code: string;
      error_fields: Record<string, unknown>;
    }>(await request.get(`${apiBaseUrl}/api/v1/sessions/${session.id}/budget-check`));
    expect(refusal).toMatchObject({
      action: "stop",
      budget_id: budget.id,
      error_code: "budget_exhausted",
      error_fields: {
        budget_id: budget.id,
        spent: 25,
        limit: 25,
        currency: "tokens",
      },
    });
    await testInfo.attach("endpoint-budget-refusal.json", {
      body: JSON.stringify(refusal, null, 2),
      contentType: "application/json",
    });

    await proxyApiToRealServer(page);
    await page.goto(`/agents/${agent.id}?tab=integrations`);
    await page.getByRole("button", { name: "Expand API endpoint details" }).click();

    await expect(page.getByRole("heading", { name: "Endpoint budget" })).toBeVisible();
    await expect(page.getByText(budget.id, { exact: true })).toBeVisible();
    await expect(page.getByText("exhausted", { exact: true })).toBeVisible();
    await expect(page.getByText("0.00 of 25.00 tokens remaining", { exact: true })).toBeVisible();
    await expect(page.getByText("1h sliding", { exact: false })).toBeVisible();
    await expect(page.getByText("Reset due", { exact: false })).toBeVisible();

    await testInfo.attach("endpoint-budget-cap.png", {
      body: await page.screenshot({ fullPage: true }),
      contentType: "image/png",
    });
  });
});
