import { expect, test, type Page, type Locator } from "@playwright/test";

const DEFAULT_ORG_ID = "org_00000000000000000000000000000001";
const AGENT_ID = "agent_019fd9b43fa37512b8f25226b21c2c8b";
const HARNESS_ID = "harness_019fd9b43fa37512b8f25226b21c2c8b";

async function mockAgentDetailApi(page: Page, displayName = "Jokes Agent") {
  await page.route("**/api/v1/**", async (route) => {
    const pathname = new URL(route.request().url()).pathname;
    let json: unknown;

    if (pathname === "/api/v1/auth/config") {
      json = {
        mode: "none",
        password_auth_enabled: false,
        oauth_providers: [],
        signup_enabled: false,
      };
    } else if (pathname === "/api/v1/auth/me") {
      json = {
        id: "user-1",
        email: "dev@example.com",
        name: "Dev User",
        roles: [],
        email_verified: true,
        organizations: [{ public_id: DEFAULT_ORG_ID, name: "Default", role: "owner" }],
      };
    } else if (pathname.endsWith("/feature-flags")) {
      json = {
        notifications: false,
        evals: true,
        plugins: true,
        channel_budgets: false,
        voice: false,
        agent_delegation: false,
        observers: true,
        public_chat: false,
      };
    } else if (pathname.endsWith("/switch-org")) {
      json = { success: true, org_id: DEFAULT_ORG_ID };
    } else if (pathname === "/api/v1/plugins") {
      json = {
        data: [
          {
            id: "plugin-test",
            name: "resend",
            display_name: "Resend",
            description: "Send transactional email",
            version: "1.0.0",
            capability_ref: "plugin:plugin-test",
            status: "active",
            warnings: [],
            identity_required: [],
            update_available: false,
            created_at: "2026-08-01T00:00:00Z",
            updated_at: "2026-08-01T00:00:00Z",
          },
        ],
        total: 1,
        has_more: false,
      };
    } else if (pathname === `/api/v1/agents/${AGENT_ID}`) {
      json = {
        id: AGENT_ID,
        name: "jokes-agent",
        harness_id: "harness_test",
        display_name: displayName,
        description:
          "A cheerful agent with a deliberately longer localized description for responsive layout testing.",
        system_prompt: "Tell a joke.",
        default_model_id: null,
        tags: [],
        capabilities: [],
        status: "active",
        session_count: 12,
        app_count: 3,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
        archived_at: null,
        deleted_at: null,
      };
    } else if (
      pathname === `/api/v1/agents/${AGENT_ID}/channels` ||
      pathname === `/api/v1/agents/${AGENT_ID}/triggers` ||
      pathname === `/api/v1/agents/${AGENT_ID}/mcp-attachments`
    ) {
      json = [];
    } else if (pathname === `/api/v1/agents/${AGENT_ID}/stats`) {
      json = {
        sessions: 12,
        messages: 24,
        input_tokens: 1200,
        output_tokens: 600,
      };
    } else if (pathname === "/api/v1/agents/config" || pathname === "/api/v1/harnesses/config") {
      json = { policies: { "agent.manage": true, "harness.manage": true } };
    } else if (pathname === `/api/v1/history/${AGENT_ID}`) {
      json = [];
    } else if (pathname === `/api/v1/context/${AGENT_ID}`) {
      json = { entity_kind: "agent", entity_ref: AGENT_ID, content: "", revision: 0 };
    } else if (pathname === `/api/v1/harnesses/${HARNESS_ID}`) {
      json = {
        id: HARNESS_ID,
        name: "responsive-harness",
        display_name: "Responsive Harness",
        description: "A representative detail page using the standard masthead action cluster.",
        system_prompt: "Help the user.",
        default_model_id: null,
        parent_harness_id: null,
        tags: [],
        capabilities: [],
        initial_files: [],
        is_built_in: false,
        status: "active",
        session_count: 4,
        app_count: 2,
        created_at: "2026-08-01T00:00:00Z",
        updated_at: "2026-08-01T00:00:00Z",
        archived_at: null,
        deleted_at: null,
      };
    } else if (pathname === `/api/v1/harnesses/${HARNESS_ID}/stats`) {
      json = { sessions: 4, messages: 8, input_tokens: 400, output_tokens: 200 };
    } else if (pathname === "/api/v1/sessions") {
      json = { data: [], has_more: false, total: 0, offset: 0, limit: 10 };
    } else if (
      pathname === "/api/v1/capabilities" ||
      pathname === "/api/v1/models" ||
      pathname === "/api/v1/harnesses"
    ) {
      json = { data: [], has_more: false, total: 0 };
    } else if (/^\/api\/v1\/orgs\/[^/]+$/.test(pathname)) {
      json = {
        id: DEFAULT_ORG_ID,
        name: "Default",
        default_model_id: null,
        default_harness_id: null,
        base_harness_id: null,
      };
    } else {
      json = { data: [], has_more: false, total: 0 };
    }

    await route.fulfill({ json });
  });
}

test.describe("Page masthead responsive layout", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    const origin = new URL(baseURL ?? "http://localhost:9100");
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: origin.hostname, path: "/" },
    ]);
    await mockAgentDetailApi(page);
  });

  test("aligns the agent title, avatar, copy action, and badges on one center line", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(`/agents/${AGENT_ID}`);
    const masthead = page.locator('[data-slot="page-masthead"]');
    await expect(masthead.getByRole("heading", { name: "Jokes Agent" })).toBeVisible();

    const centers = await masthead.evaluate((element) => {
      const selectors = [
        '[data-slot="icon-tile"]',
        '[data-slot="entity-identity-label"]',
        'button[aria-label^="Copy ID:"]',
        ".font-mono",
        '[data-slot="badge"]',
      ];
      return selectors.map((selector) => {
        const box = element.querySelector(selector)!.getBoundingClientRect();
        return box.y + box.height / 2;
      });
    });
    expect(Math.max(...centers) - Math.min(...centers)).toBeLessThanOrEqual(1);
  });

  test("wraps a long agent title while keeping its copy action contained on mobile", async ({
    page,
  }) => {
    const displayName = "A deliberately long readable agent heading for a narrow viewport";
    await mockAgentDetailApi(page, displayName);
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`/agents/${AGENT_ID}`);

    const title = page.getByRole("heading", { name: displayName });
    const copy = title.getByRole("button", { name: `Copy ID: ${AGENT_ID}` });
    await expect(title).toBeVisible();
    await expect(copy).toBeVisible();
    const titleBox = (await title.boundingBox())!;
    const copyBox = (await copy.boundingBox())!;

    expect(titleBox.height).toBeGreaterThan(40);
    expect(copyBox.x + copyBox.width).toBeLessThanOrEqual(titleBox.x + titleBox.width);
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      390,
    );
  });

  // The agent page keeps a three-action header at every width (Edit, the
  // overflow menu, Test in Playground); secondary actions live in the overflow.
  test("keeps the agent actions contained with secondary actions in the overflow at mobile width", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`/agents/${AGENT_ID}`);

    const title = page.getByRole("heading", { name: "Jokes Agent" });
    const masthead = title.locator("xpath=ancestor::div[@data-slot='page-masthead'][1]");
    const actions = masthead.locator('[data-slot="page-masthead-actions"]');
    const moreActions = page.getByRole("button", { name: "More actions" });
    await expect(title).toBeVisible();
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Test in Playground" })).toBeVisible();
    await expect(page.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
    await expect(moreActions).toBeVisible();
    await expect(page.getByRole("button", { name: "Copy", exact: true })).toHaveCount(0);

    await page.getByRole("button", { name: "Open navigation" }).click();
    await expect(page.locator('[data-slot="drawer-content"]')).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page.locator('[data-slot="drawer-content"]')).toBeHidden();

    await moreActions.click({ trial: true });
    await moreActions.click();
    await expect(page.getByRole("menuitem", { name: "Copy" })).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: "Export package (ZIP)", exact: true }),
    ).toBeVisible();
    await expect(
      page.getByRole("menuitem", { name: "Export Markdown", exact: true }),
    ).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Observe this agent" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "History", exact: true })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Manager notes" })).toBeVisible();
    await expect(page.getByRole("menuitem", { name: "Version history" })).toHaveCount(0);
    await expect(page.getByRole("menuitem", { name: "Archive agent" })).toBeVisible();

    await page.keyboard.press("Escape");
    await expect(moreActions).toBeFocused();
    await moreActions.press("Enter");
    await expect(page.getByRole("menuitem", { name: "Copy" })).toBeVisible();
    await page.keyboard.press("Escape");

    const mastheadBox = await masthead.boundingBox();
    const actionsBox = await actions.boundingBox();
    expect(mastheadBox).not.toBeNull();
    expect(actionsBox).not.toBeNull();
    expect(actionsBox!.x + actionsBox!.width).toBeLessThanOrEqual(
      mastheadBox!.x + mastheadBox!.width,
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
  });

  // History and Manager notes open as sheets from the entity actions menu, and
  // the open sheet is part of the address so a reload reopens it.
  test("opens History and Manager notes from the agent menu and keeps them in the URL", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 1280, height: 900 });
    await page.goto(`/agents/${AGENT_ID}`);

    const moreActions = page.getByRole("button", { name: "More actions for Jokes Agent" });
    await moreActions.focus();
    await page.keyboard.press("Enter");
    await page.getByRole("menuitem", { name: "History", exact: true }).click();
    await expect(page).toHaveURL(/[?&]sheet=history/);
    await expect(page.getByText("No changes recorded yet.")).toBeVisible();
    await page.reload();
    await expect(page.getByText("No changes recorded yet.")).toBeVisible();
    await page.keyboard.press("Escape");
    await expect(page).not.toHaveURL(/sheet=/);

    await moreActions.click();
    await page.getByRole("menuitem", { name: "Manager notes" }).click();
    await expect(page).toHaveURL(/[?&]sheet=notes/);
    await expect(page.getByText("No manager notes yet")).toBeVisible();
    await page.reload();
    await expect(page.getByText("No manager notes yet")).toBeVisible();
  });

  test("opens History for retired ?tab=versions links", async ({ page }) => {
    await page.goto(`/agents/${AGENT_ID}?tab=versions`);
    await expect(page).toHaveURL(/[?&]sheet=history/);
    await expect(page.getByText("No changes recorded yet.")).toBeVisible();
  });

  for (const viewport of [
    { name: "compact desktop", width: 1024, height: 900 },
    { name: "tablet", width: 768, height: 900 },
  ]) {
    test(`keeps the agent actions visible and contained at ${viewport.name} width`, async ({
      page,
    }) => {
      await page.setViewportSize({ width: viewport.width, height: viewport.height });
      await page.goto(`/agents/${AGENT_ID}`);

      const title = page.getByRole("heading", { name: "Jokes Agent" });
      const masthead = title.locator("xpath=ancestor::div[@data-slot='page-masthead'][1]");
      const actions = masthead.locator('[data-slot="page-masthead-actions"]');

      await expect(page.getByRole("button", { name: "Test in Playground" })).toBeVisible();
      await expect(page.getByRole("button", { name: "Edit", exact: true })).toBeVisible();
      await expect(page.getByRole("button", { name: "More actions" })).toBeVisible();

      const mastheadBox = await masthead.boundingBox();
      const actionsBox = await actions.boundingBox();
      expect(mastheadBox).not.toBeNull();
      expect(actionsBox).not.toBeNull();
      expect(actionsBox!.x + actionsBox!.width).toBeLessThanOrEqual(
        mastheadBox!.x + mastheadBox!.width,
      );
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        await page.evaluate(() => document.documentElement.clientWidth),
      );
    });
  }

  test("keeps actions aligned beside identity content at wide desktop width", async ({ page }) => {
    await page.setViewportSize({ width: 1440, height: 1000 });
    await page.goto(`/agents/${AGENT_ID}`);

    const title = page.getByRole("heading", { name: "Jokes Agent" });
    const actions = page
      .getByRole("button", { name: "Test in Playground" })
      .locator("xpath=ancestor::div[@data-slot='page-masthead-actions'][1]");
    const titleBox = await title.boundingBox();
    const actionsBox = await actions.boundingBox();

    expect(titleBox).not.toBeNull();
    expect(actionsBox).not.toBeNull();
    expect(actionsBox!.y).toBeLessThan(titleBox!.y + titleBox!.height);
    await expect(page.getByRole("button", { name: "Open navigation" })).toBeHidden();
    await expect(page.locator('[data-slot="page-masthead"] a > button')).toHaveCount(0);
  });

  test("keeps a representative standard-action consumer contained on mobile", async ({ page }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    await page.goto(`/harnesses/${HARNESS_ID}`);

    const title = page.getByRole("heading", { name: "Responsive Harness" });
    const masthead = title.locator("xpath=ancestor::div[@data-slot='page-masthead'][1]");
    const edit = page.getByRole("link", { name: "Edit" });

    await expect(title).toBeVisible();
    await expect(page.getByRole("link", { name: "Create app" })).toHaveCount(0);
    await expect(page.getByRole("button", { name: "Copy", exact: true })).toBeVisible();
    await expect(edit).toBeVisible();
    await expect(
      page.getByRole("button", { name: "More actions for Responsive Harness" }),
    ).toBeVisible();
    await edit.click({ trial: true });
    await expect(masthead.locator("a > button")).toHaveCount(0);

    const mastheadBox = await masthead.boundingBox();
    expect(mastheadBox).not.toBeNull();
    expect(mastheadBox!.x + mastheadBox!.width).toBeLessThanOrEqual(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
    expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
      await page.evaluate(() => document.documentElement.clientWidth),
    );
  });

  for (const width of [390, 1440]) {
    test(`keeps plugin overview counts and cards contained at ${width}px`, async ({ page }) => {
      await page.setViewportSize({ width, height: 900 });
      await page.goto("/plugins");
      await expect(page.getByText("Send transactional email")).toBeVisible();
      const installed = page.getByRole("tab", { name: "Installed Plugins", exact: true });
      await expect(installed).toHaveAttribute("aria-selected", "true");
      await expect(installed).toContainText("1");
      await expect(
        page.locator('[data-slot="page-masthead"]').getByText("1", { exact: true }),
      ).toHaveCount(0);
      await expect(page.getByRole("button", { name: "Copy ID: plugin:plugin-test" })).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
      await page.getByRole("tab", { name: "Marketplaces", exact: true }).click();
      await expect(
        page
          .locator('[data-slot="page-masthead"]')
          .getByRole("button", { name: "Add Marketplace" }),
      ).toBeVisible();
      expect(await page.evaluate(() => document.documentElement.scrollWidth)).toBeLessThanOrEqual(
        width,
      );
    });
  }
});

// Sample painted frames: jsdom cannot detect missing CSS or a blank exit frame.
test.describe("Agent settings drawer animation", () => {
  test.beforeEach(async ({ context, page, baseURL }) => {
    await context.addCookies([
      { name: "access_token", value: "e2e-only", domain: new URL(baseURL!).hostname, path: "/" },
    ]);
    await mockAgentDetailApi(page);
    await page.setViewportSize({ width: 1440, height: 900 });
    await page.goto(`/agents/${AGENT_ID}`);
    await expect(page.getByRole("heading", { name: "Jokes Agent" })).toBeVisible();
  });

  for (const theme of ["light", "dark"]) {
    test(`animates settings without blank or resized exit frames in ${theme} mode`, async ({
      page,
    }, testInfo) => {
      await page.evaluate(
        (dark) => document.documentElement.classList.toggle("dark", dark),
        theme === "dark",
      );
      for (const title of [
        "Primary sandbox",
        "Network access",
        "Files",
        "Token usage",
        "MCP servers",
        "Credentials",
        "Health check",
        "Branding",
      ]) {
        const row = page.getByRole("button", { name: new RegExp(`^${title}`) });
        await expect(row).toBeVisible();
        const opening = await sampleDrawerFrames(row);
        await testInfo.attach(`${title}-opening`, {
          body: JSON.stringify(opening),
          contentType: "application/json",
        });
        expect(
          opening.some((frame) => frame.opacity > 0 && frame.opacity < 1),
          `${title} should have intermediate opening frames`,
        ).toBe(true);
        expect(opening.some((frame) => frame.overlayOpacity > 0 && frame.overlayOpacity < 1)).toBe(
          true,
        );
        expect(
          opening.every(
            (frame, index) => index === 0 || frame.opacity >= opening[index - 1].opacity,
          ),
        ).toBe(true);
        const drawer = page.locator('[data-slot="drawer-content"]');
        await expect(drawer.getByRole("heading", { name: title, exact: true })).toBeVisible();
        const width = (await drawer.boundingBox())!.width;
        const closing = await sampleDrawerFrames(
          drawer.getByRole("button", { name: "Close", exact: true }),
        );
        await testInfo.attach(`${title}-closing`, {
          body: JSON.stringify(closing),
          contentType: "application/json",
        });
        const visible = closing.filter((frame) => frame.opacity > 0);
        expect(
          visible.some((frame) => frame.opacity < 1),
          `${title} should have intermediate closing frames`,
        ).toBe(true);
        expect(
          visible.every((frame) => frame.title === title),
          `${title} should retain its contents until hidden`,
        ).toBe(true);
        expect(
          visible.every((frame) => Math.abs(frame.width - width) < 1),
          `${title} should retain its width until hidden`,
        ).toBe(true);
        expect(
          closing.every(
            (frame, index) => index === 0 || frame.opacity <= closing[index - 1].opacity,
          ),
        ).toBe(true);
        expect(closing.some((frame) => frame.overlayOpacity > 0 && frame.overlayOpacity < 1)).toBe(
          true,
        );
        await expect(drawer).toHaveCount(0);
      }
    });
  }

  test("animates the mobile navigation and supports Done, Escape, and backdrop dismissal", async ({
    page,
  }) => {
    await page.setViewportSize({ width: 390, height: 844 });
    const drawer = page.locator('[data-slot="drawer-content"]');
    const opening = await sampleDrawerFrames(page.getByRole("button", { name: "Open navigation" }));
    expect(opening.some((frame) => frame.opacity > 0 && frame.opacity < 1)).toBe(true);
    await page.keyboard.press("Escape");
    await expect(drawer).toHaveCount(0);
    for (const dismissal of ["Done", "Escape", "backdrop"]) {
      await page.getByRole("button", { name: /^Primary sandbox/ }).click();
      await expect(
        drawer.getByRole("heading", { name: "Primary sandbox", exact: true }),
      ).toBeVisible();
      if (dismissal === "Done")
        await drawer.getByRole("button", { name: "Done", exact: true }).click();
      else if (dismissal === "Escape") await page.keyboard.press("Escape");
      else {
        // At mobile width the popup fills the screen; expose the backdrop at desktop width.
        await page.setViewportSize({ width: 1440, height: 900 });
        await page.locator('[data-slot="drawer-overlay"]').click({ position: { x: 10, y: 100 } });
      }
      await expect(drawer).toHaveCount(0);
    }
  });

  test("respects reduced motion when opening and closing settings", async ({ page }) => {
    await page.emulateMedia({ reducedMotion: "reduce" });
    const opening = await sampleDrawerFrames(
      page.getByRole("button", { name: /^Primary sandbox/ }),
    );
    expect(opening.some((frame) => frame.title === "Primary sandbox" && frame.opacity === 1)).toBe(
      true,
    );
    expect(opening.every((frame) => frame.opacity === 0 || frame.opacity === 1)).toBe(true);
    const drawer = page.locator('[data-slot="drawer-content"]');
    expect(await drawer.evaluate((element) => getComputedStyle(element).transitionProperty)).toBe(
      "none",
    );
    await drawer.getByRole("button", { name: "Close", exact: true }).click();
    await expect(drawer).toHaveCount(0);
  });

  test("can close partway through opening without a blank frame", async ({ page }) => {
    const frames = await sampleDrawerFrames(
      page.getByRole("button", { name: /^Primary sandbox/ }),
      true,
    );
    const visible = frames.filter((frame) => frame.opacity > 0);
    expect(visible.length).toBeGreaterThan(1);
    expect(visible.every((frame) => frame.title === "Primary sandbox")).toBe(true);
    expect(visible.every((frame) => Math.abs(frame.width - visible[0].width) < 1)).toBe(true);
    await expect(page.locator('[data-slot="drawer-content"]')).toHaveCount(0);
  });
});

async function sampleDrawerFrames(trigger: Locator, closeDuringOpening = false) {
  return trigger.evaluate(async (button, closeDuringOpening) => {
    const frames: {
      opacity: number;
      width: number;
      title: string | null;
      overlayOpacity: number;
    }[] = [];
    (button as HTMLButtonElement).click();
    let openingFrames = 0;
    const started = performance.now();
    await new Promise<void>((resolve) => {
      const sample = () => {
        const drawer = document.querySelector<HTMLElement>('[data-slot="drawer-content"]');
        const overlay = document.querySelector<HTMLElement>('[data-slot="drawer-overlay"]');
        frames.push({
          opacity: drawer ? Number(getComputedStyle(drawer).opacity) : 0,
          width: drawer?.getBoundingClientRect().width ?? 0,
          title: drawer?.querySelector('[data-slot="drawer-title"]')?.textContent ?? null,
          overlayOpacity: overlay ? Number(getComputedStyle(overlay).opacity) : 0,
        });
        // A wall-clock timer can fire before CI paints the first visible frame.
        // Interrupt only after observing the opening transition in progress.
        if (closeDuringOpening && frames.at(-1)!.opacity > 0 && frames.at(-1)!.opacity < 1) {
          openingFrames += 1;
          if (openingFrames === 2) {
            document.querySelector<HTMLButtonElement>('[data-slot="drawer-close"]')?.click();
          }
        }
        if (performance.now() - started < 400) requestAnimationFrame(sample);
        else resolve();
      };
      requestAnimationFrame(sample);
    });
    return frames;
  }, closeDuringOpening);
}
