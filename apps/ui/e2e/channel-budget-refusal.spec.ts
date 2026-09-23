import { execFileSync } from "node:child_process";
import { createHash, randomUUID } from "node:crypto";
import { expect, test, type APIResponse, type Page } from "@playwright/test";

const apiBaseUrl = process.env.PLAYWRIGHT_REAL_API_URL;
const databaseUrl = process.env.DATABASE_URL;
const agentPrompt = "Budget metering system prompt now.";
const scheduleMessage = "Consume twenty-five tokens exactly.";
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
function runSql(sql: string, variables: Record<string, string> = {}): string {
  const args = [
    databaseUrl!,
    "--no-psqlrc",
    "--set",
    "ON_ERROR_STOP=1",
    "--tuples-only",
    "--no-align",
  ];
  for (const [name, value] of Object.entries(variables)) {
    args.push("--set", `${name}=${value}`);
  }
  args.push("--file=-");
  return execFileSync("psql", args, { encoding: "utf8", input: sql }).trim();
}

async function pollFor<T>(
  fetchValue: () => Promise<T>,
  matches: (value: T) => boolean,
  description: string,
): Promise<T> {
  const deadline = Date.now() + 60_000;
  let lastValue: T | undefined;
  while (Date.now() < deadline) {
    lastValue = await fetchValue();
    if (matches(lastValue)) return lastValue;
    await new Promise((resolve) => setTimeout(resolve, 500));
  }
  throw new Error(`Timed out waiting for ${description}: ${JSON.stringify(lastValue)}`);
}

test.describe("channel budget refusal", () => {
  test.skip(
    !apiBaseUrl || !databaseUrl,
    "Requires PLAYWRIGHT_REAL_API_URL and DATABASE_URL for the real PostgreSQL-backed server.",
  );

  test("shows the cap whose stable ID appears in a real refusal", async ({
    page,
    request,
  }, testInfo) => {
    const suffix = randomUUID();
    const provider = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/providers`, {
        data: {
          name: `Channel budget llmsim ${suffix}`,
          provider_type: "llmsim",
        },
      }),
    );
    const model = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/providers/${provider.id}/models`, {
        data: {
          model_id: `channel-budget-${suffix}`,
          display_name: "Channel budget llmsim",
          enabled: true,
        },
      }),
    );
    const harness = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/harnesses`, {
        data: {
          name: `budget-e2e-${suffix}`,
          display_name: "Channel budget E2E",
          system_prompt: "",
        },
      }),
    );
    const agent = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/agents`, {
        data: {
          name: `budget-e2e-${suffix}`,
          display_name: "Channel budget refusal",
          system_prompt: agentPrompt,
          harness_id: harness.id,
          default_model_id: model.id,
        },
      }),
    );

    const apiKey = `evr_app_${randomUUID().replaceAll("-", "")}`;
    const bootstrapChannel = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/agents/${agent.id}/channels`, {
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
    const channelUuid = randomUUID();
    const channelId = `appchan_${channelUuid.replaceAll("-", "")}`;
    const seededChannel = runSql(
      `WITH seeded AS (
         INSERT INTO agent_channels (
           id, agent_id, app_id, legacy_app_public_id, public_id, channel_type,
           channel_config, enabled, status, agent_identity_id, agent_version_policy,
           agent_version_id, owner_principal_id, resolved_owner_user_id
         )
         SELECT
           :'channel_uuid'::uuid, agent_id, NULL, NULL, :'channel_id', 'schedule',
           :'channel_config'::jsonb, true, 'live', agent_identity_id, agent_version_policy,
           agent_version_id, owner_principal_id, resolved_owner_user_id
         FROM agent_channels
         WHERE public_id = :'bootstrap_channel_id'
         RETURNING public_id
       )
       SELECT public_id FROM seeded;`,
      {
        channel_uuid: channelUuid,
        channel_id: channelId,
        bootstrap_channel_id: bootstrapChannel.id,
        channel_config: JSON.stringify({
          cron_expression: "0 0 * * * * *",
          timezone: "UTC",
          session_mode: "session_per_invocation",
          message: scheduleMessage,
        }),
      },
    );
    expect(seededChannel).toBe(channelId);

    const budget = await jsonResponse<{ id: string }>(
      await request.post(`${apiBaseUrl}/api/v1/budgets`, {
        data: {
          subject_type: "agent_channel",
          subject_id: channelId,
          currency: "tokens",
          limit: 25,
          period: { type: "duration", seconds: 3600 },
        },
      }),
    );
    const consumed = await jsonResponse<{ session_id: string; created_session: boolean }>(
      await request.post(`${apiBaseUrl}/api/v1/agents/${agent.id}/channels/${channelId}/trigger`),
    );
    expect(consumed.created_session).toBe(true);
    expect(
      runSql(
        `SELECT channel.public_id
         FROM sessions AS session
         JOIN agent_channels AS channel ON channel.id = session.channel_id
         WHERE replace(session.id::text, '-', '') = substring(:'session_id' from 9);`,
        { session_id: consumed.session_id },
      ),
    ).toBe(channelId);

    const exhausted = await pollFor(
      async () =>
        jsonResponse<{ balance: number; status: string }>(
          await request.get(`${apiBaseUrl}/api/v1/budgets/${budget.id}`),
        ),
      (current) => current.status === "exhausted",
      "the production budget listener to exhaust the channel budget",
    );
    expect(exhausted.balance).toBe(0);

    const ledger = await jsonResponse<
      Array<{
        amount: number;
        meter_source: string;
        ref_type: string;
        session_id: string;
      }>
    >(await request.get(`${apiBaseUrl}/api/v1/budgets/${budget.id}/ledger`));
    expect(ledger).toEqual([
      expect.objectContaining({
        amount: 25,
        meter_source: "llm_tokens",
        ref_type: "llm_generation",
        session_id: consumed.session_id,
      }),
    ]);

    const refused = await jsonResponse<{ session_id: string; created_session: boolean }>(
      await request.post(`${apiBaseUrl}/api/v1/agents/${agent.id}/channels/${channelId}/trigger`),
    );
    expect(refused.created_session).toBe(true);
    expect(refused.session_id).not.toBe(consumed.session_id);
    expect(
      runSql(
        `SELECT channel.public_id
         FROM sessions AS session
         JOIN agent_channels AS channel ON channel.id = session.channel_id
         WHERE replace(session.id::text, '-', '') = substring(:'session_id' from 9);`,
        { session_id: refused.session_id },
      ),
    ).toBe(channelId);

    const refusal = await jsonResponse<{
      action: string;
      budget_id: string;
      error_code: string;
      error_fields: Record<string, unknown>;
    }>(await request.get(`${apiBaseUrl}/api/v1/sessions/${refused.session_id}/budget-check`));
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
    await testInfo.attach("channel-budget-refusal.json", {
      body: JSON.stringify({ consumed, ledger, refused, refusal }, null, 2),
      contentType: "application/json",
    });

    await proxyApiToRealServer(page);
    await page.goto(`/agents/${agent.id}?tab=integrations`);
    await page.getByRole("button", { name: `Expand ${scheduleMessage} details` }).click();

    const channelBudgetPanel = page
      .getByRole("heading", { name: "Channel budget", exact: true })
      .locator("xpath=../..");
    await expect(channelBudgetPanel).toBeVisible();
    await expect(channelBudgetPanel.getByText(budget.id, { exact: true })).toBeVisible();
    await expect(channelBudgetPanel.getByText("exhausted", { exact: true })).toBeVisible();
    await expect(
      channelBudgetPanel.getByText("0.00 of 25.00 tokens remaining", { exact: true }),
    ).toBeVisible();
    await expect(channelBudgetPanel.getByText("1h sliding", { exact: false })).toBeVisible();
    await expect(channelBudgetPanel.getByText("Reset due", { exact: false })).toBeVisible();

    await testInfo.attach("channel-budget-cap.png", {
      body: await channelBudgetPanel.screenshot(),
      contentType: "image/png",
    });
  });
});
