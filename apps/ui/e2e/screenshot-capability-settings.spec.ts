import { test } from '@playwright/test';
import { readFileSync } from 'fs';

test('capability settings screenshots', async ({ page }) => {
  const agentId = readFileSync('/tmp/test_agent_id.txt', 'utf-8').trim();
  console.log('Agent ID:', agentId);

  // 1. Go to agent edit page
  console.log('Loading agent edit page...');
  await page.goto(`/agents/${agentId}/edit`);
  await page.waitForLoadState('networkidle');
  await page.waitForTimeout(2000);

  // Take screenshot of initial state (showing capability list with Docker selected)
  await page.screenshot({ path: 'e2e/screenshots/capability-settings-1-initial.png' });
  console.log('Screenshot 1: Agent edit page with Docker capability');

  // 2. Click the settings gear icon to expand Docker capability settings
  console.log('Expanding Docker capability settings...');
  const settingsButton = page.locator('button[aria-label="Toggle settings"]');

  if (await settingsButton.count() > 0) {
    await settingsButton.click();
    await page.waitForTimeout(500);

    // Take screenshot with settings expanded
    await page.screenshot({ path: 'e2e/screenshots/capability-settings-2-expanded.png' });
    console.log('Screenshot 2: Docker capability settings expanded');

    // 3. Enter a custom image
    console.log('Entering custom Docker image...');
    const imageInput = page.locator('input#docker-image');
    await imageInput.fill('node:20-alpine');
    await page.waitForTimeout(300);

    // Take screenshot with custom image
    await page.screenshot({ path: 'e2e/screenshots/capability-settings-3-custom-image.png' });
    console.log('Screenshot 3: Custom Docker image entered');
  } else {
    console.log('Settings button not found');
    await page.screenshot({ path: 'e2e/screenshots/capability-settings-debug.png', fullPage: true });
  }
});
