import { test, expect } from '@playwright/test';

/**
 * E2E tests for Message Components via Dev Pages.
 *
 * Tests verify message rendering, tool calls, and todo list components.
 * Uses /dev/components page which is only available in development mode.
 */

test.describe('Message Components', () => {
  test.beforeEach(async ({ page }) => {
    await page.goto('/dev/components');
  });

  test('should render the page title', async ({ page }) => {
    await expect(page.getByRole('heading', { name: 'Session Chat Components' })).toBeVisible();
  });

  test('should render message showcases', async ({ page }) => {
    // Check section headers
    await expect(page.getByRole('heading', { name: 'Message Rendering' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'ToolCallCard Component' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'TodoListRenderer Component' })).toBeVisible();
    await expect(page.getByRole('heading', { name: 'Combined Chat View' })).toBeVisible();
  });

  test('should render user and assistant messages', async ({ page }) => {
    // User message should be visible
    await expect(page.getByText('Hello! Can you help me analyze this code?')).toBeVisible();

    // Assistant message should be visible
    await expect(page.getByText("I'll help you with that")).toBeVisible();
  });

  test('should render tool call cards', async ({ page }) => {
    // Tool names should be visible
    await expect(page.getByText('list_files').first()).toBeVisible();
    await expect(page.getByText('bash').first()).toBeVisible();
    await expect(page.getByText('read_file').first()).toBeVisible();
  });

  test('should render todo list with different states', async ({ page }) => {
    // Completed, in-progress, and pending tasks should be visible
    await expect(page.getByText('Review code changes').first()).toBeVisible();
    await expect(page.getByText('Run tests').first()).toBeVisible();
  });

  test('should take full page screenshot', async ({ page }) => {
    // Wait for all content to load
    await page.waitForLoadState('networkidle');

    // Take screenshot for visual regression testing
    await page.screenshot({
      path: 'e2e/screenshots/message-components-full.png',
      fullPage: true,
    });
  });

  test('should take section screenshots', async ({ page }) => {
    await page.waitForLoadState('networkidle');

    // Screenshot of Message Rendering section
    const messageSection = page.getByRole('heading', { name: 'Message Rendering' }).locator('..');
    if (await messageSection.isVisible()) {
      await messageSection.screenshot({
        path: 'e2e/screenshots/message-components-messages.png',
      });
    }

    // Screenshot of ToolCallCard section
    const toolSection = page.getByRole('heading', { name: 'ToolCallCard Component' }).locator('..');
    if (await toolSection.isVisible()) {
      await toolSection.screenshot({
        path: 'e2e/screenshots/message-components-toolcalls.png',
      });
    }
  });
});

test.describe('Dev Index Page', () => {
  test('should render developer tools index', async ({ page }) => {
    await page.goto('/dev');

    await expect(page.getByRole('heading', { name: 'Developer Tools' })).toBeVisible();
    await expect(page.getByText('Development Mode')).toBeVisible();
  });

  test('should navigate to components page', async ({ page }) => {
    await page.goto('/dev');

    await page.getByRole('link', { name: /Session Chat Components/i }).click();

    await expect(page).toHaveURL('/dev/components');
    await expect(page.getByRole('heading', { name: 'Session Chat Components' })).toBeVisible();
  });
});
