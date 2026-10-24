import { test, expect } from "@playwright/test";

test.use({ deviceScaleFactor: 2 });
const orgId = "org_00000000000000000000000000000001";
const person = (id: string, name: string, status = "active") => ({
  id,
  name,
  status,
  usage: "end_user",
});

test("searches Talk as and counterparts with keyboard, paging, and stable selection", async ({
  page,
  context,
  baseURL,
}) => {
  await context.addCookies([
    { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
  ]);
  let isAdmin = true;
  let sessionWrites = 0;
  const searches: string[] = [];
  await page.route("**/api/v1/**", async (route) => {
    const request = route.request();
    const url = new URL(request.url());
    const path = url.pathname.replace("/api/v1", "");
    let json: unknown = { data: [], total: 0, has_more: false };
    if (path === "/auth/config")
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    else if (path === "/auth/me")
      json = {
        id: "user-1",
        email: "dev@example.com",
        name: "Dev",
        email_verified: true,
        roles: [],
        organizations: [{ public_id: orgId, name: "Default", role: isAdmin ? "owner" : "member" }],
      };
    else if (path.endsWith("/feature-flags")) json = {};
    else if (path.endsWith("/switch-org")) json = { success: true, org_id: orgId };
    else if (path === `/orgs/${orgId}`) json = { id: orgId, name: "Default" };
    else if (path === "/virtual-users/me" || path === "/virtual-users/person_alice")
      json = person("person_alice", "Alice Rivers");
    else if (path === "/virtual-users/person_zara") json = person("person_zara", "Zara Quinn");
    else if (path === "/virtual-users") {
      expect(url.searchParams.get("usage")).toBe("end_user");
      const search = url.searchParams.get("search") ?? "";
      searches.push(search);
      const offset = Number(url.searchParams.get("offset"));
      if (search === "fail") {
        await route.fulfill({ status: 500, json: { message: "Unavailable" } });
        return;
      }
      const matches = search.toLowerCase().includes("zara")
        ? [person("person_zara", "Zara Quinn")]
        : search
          ? []
          : offset
            ? [person("person_zara", "Zara Quinn")]
            : [
                person("person_alice", "Alice Rivers"),
                person("archived", "Archived Person", "archived"),
                ...Array.from({ length: 98 }, (_, index) =>
                  person(`person_${index}`, `Person ${index}`),
                ),
              ];
      json = { data: matches, total: search ? matches.length : 101, offset, limit: 100 };
    } else if (path === "/agents")
      json = {
        data: [
          { id: "agent_support", name: "support", display_name: "Support Agent", status: "active" },
        ],
        total: 1,
      };
    else if (path === "/harnesses")
      json = {
        data: [{ id: "harness_base", name: "base", display_name: "Base Harness" }],
        total: 1,
      };
    else if (path === "/llm-models")
      json = {
        data: [
          { id: "model_1", display_name: "Test Model", enabled: true, capabilities: ["chat"] },
        ],
        total: 1,
      };
    else if (path === "/sessions" && request.method() === "POST") {
      sessionWrites++;
      throw new Error("This test must never start a chat");
    }
    await route.fulfill({ json });
  });
  await page.goto("/playground/new?agent=agent_support&dry_run=1");
  await expect(page.getByRole("checkbox", { name: /Dry run/ })).toBeChecked();
  const talk = page.getByRole("combobox", { name: "Talk as virtual user", exact: true });
  await expect(talk).toHaveText("Alice Rivers");
  await talk.focus();
  await page.keyboard.press("Enter");
  const search = page.getByRole("combobox", { name: "Search virtual users", exact: true });
  await expect(search).toBeFocused();
  await expect(page.getByRole("option", { name: "Archived Person (archived)" })).toBeDisabled();
  await page.getByRole("button", { name: "Load more virtual users" }).click();
  await expect(page.getByRole("option", { name: "Zara Quinn" })).toBeVisible();
  await search.fill("nobody");
  await expect(page.getByRole("status")).toHaveText("No results found");
  await expect(talk).toHaveText("Alice Rivers");
  await search.fill("Zara");
  await expect(page.getByRole("option", { name: "Zara Quinn" })).toBeVisible();
  await search.press("ArrowDown");
  await search.press("Enter");
  await expect(talk).toHaveText("Zara Quinn");
  await expect(talk).toBeFocused();
  await talk.click();
  await expect(search).toHaveValue("");
  await search.fill("fail");
  await expect(
    page.getByRole("alert").filter({ hasText: "Could not load virtual users" }),
  ).toHaveText("Could not load virtual users. Try searching again.", { timeout: 15000 });
  await search.press("Escape");
  await expect(talk).toHaveText("Zara Quinn");
  await talk.click();
  await search.fill("Zara");
  await expect(page.getByRole("option", { name: "Zara Quinn" })).toBeVisible();
  if (process.env.UI_EVIDENCE_DIR) {
    const box = await talk.boundingBox();
    await page.screenshot({
      path: `${process.env.UI_EVIDENCE_DIR}/playground-search.png`,
      clip: { x: box!.x - 8, y: box!.y - 30, width: box!.width + 16, height: 150 },
    });
  }
  await search.press("Escape");
  await page.getByRole("combobox", { name: "Chat counterpart", exact: true }).click();
  const counterpartSearch = page.getByRole("combobox", { name: "Search agents and harnesses" });
  await counterpartSearch.fill("Base");
  await expect(page.getByRole("option", { name: "Base Harness" })).toBeVisible();
  await expect(page.getByRole("option", { name: "Support Agent" })).toHaveCount(0);
  await counterpartSearch.press("Escape");
  expect(searches).toContain("Zara");
  expect(sessionWrites).toBe(0);
  isAdmin = false;
  await page.reload();
  await expect(talk).toBeDisabled();
  await expect(talk).toHaveText("Alice Rivers");
  await expect(
    page.getByText("The agent and virtual user stay fixed for this chat."),
  ).toBeVisible();
});
