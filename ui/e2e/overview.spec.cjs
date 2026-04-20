const { expect, test } = require('@playwright/test');
const { spawn } = require('node:child_process');
const { cpSync, mkdtempSync } = require('node:fs');
const { tmpdir } = require('node:os');
const { join } = require('node:path');

test.describe.configure({ mode: 'serial' });

let readonlyServer = null;

test.beforeAll(async () => {
  const fixtureCopy = mkdtempSync(join(tmpdir(), 'dirbase-overview-readonly-'));
  const serverFolder = join(fixtureCopy, 'fixtures');
  cpSync(join(process.cwd(), 'e2e/fixtures'), serverFolder, { recursive: true });

  readonlyServer = spawn(
    'cargo',
    ['run', '--', '--folder', serverFolder, '--bind', '127.0.0.1:4511', '--readonly'],
    {
      cwd: '..',
      stdio: 'pipe'
    }
  );

  await waitForServer('http://127.0.0.1:4511');
});

test.afterAll(async () => {
  readonlyServer?.kill('SIGTERM');
});

test('supports first-load discovery, filter chips, and relation drill-down in both directions', async ({
  page
}) => {
  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await expect(page.getByTestId('resource-sidebar')).toContainText('members', { timeout: 15_000 });
  await expect(page.getByTestId('resource-sidebar')).toContainText('teams');
  await expect(page.getByTestId('resource-sidebar')).toContainText('settings');
  await expect(page.locator('.request-path').first()).toContainText('/members?page=1&per_page=25');

  await page.getByRole('button', { name: 'Add filter' }).click();
  await page.locator('.filter-row select').first().selectOption('code');
  await page.locator('.filter-row input').fill('MEM-012');
  await expect(page.getByTestId('query-summary')).toContainText('code');
  await expect(page.getByTestId('query-summary')).toContainText('equals MEM-012');
  await expect(page.locator('.request-path').first()).toContainText('code=MEM-012');

  await page.getByRole('button', { name: 'Remove filter on code' }).click();
  await expect(page.getByTestId('query-summary')).toContainText('No active filters');

  await page.locator('tbody tr').first().click();
  await expect(page.getByTestId('inspector-panel')).toContainText('Selected row');
  await page.locator('.relation-link-button').filter({ hasText: 'teams' }).first().click();
  await expect(page.locator('main').getByRole('heading', { name: 'teams' })).toBeVisible();
  await expect(page.locator('.request-path').first()).toContainText('/teams?page=1&per_page=25&id=1');

  await page.locator('tbody tr').first().click();
  await page.locator('.relation-link-button').filter({ hasText: 'members' }).first().click();
  await expect(page.locator('main').getByRole('heading', { name: 'members' })).toBeVisible();
  await expect(page.locator('.request-path').first()).toContainText('team_id=1');
});

test('supports mobile drawers and inspector sheet without losing explorer state', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await page.getByRole('button', { name: 'Resources' }).click();
  await expect(page.getByTestId('resource-sidebar')).toHaveClass(/mobile-drawer-open/);

  await page.getByRole('button', { name: 'Map' }).click();
  await expect(page.locator('.relation-map-panel')).toHaveClass(/mobile-drawer-open/);

  await page.getByRole('button', { name: 'Map' }).click();
  await page.locator('tbody tr').first().click();
  await expect(page.getByTestId('inspector-panel')).toHaveClass(/mobile-sheet-open/);
  await expect(page.locator('main').getByRole('heading', { name: 'members' })).toBeVisible();
});

test('hides write actions in read-only mode', async ({ page }) => {
  await page.goto('http://127.0.0.1:4511/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await expect(page.getByText('Read-only mode')).toBeVisible();
  await expect(page.getByRole('button', { name: 'New row' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Delete row' })).toHaveCount(0);

  await page.getByRole('button', { name: 'Schema' }).click();
  await expect(page.getByRole('button', { name: 'Infer from data' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled();
});

test('supports object raw mode and invalid mutation feedback', async ({ page }) => {
  await page.goto('/?resource=settings');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await expect(page.locator('main').getByRole('heading', { name: 'settings' })).toBeVisible();
  await page.getByRole('button', { name: 'Raw JSON' }).click();
  await expect(page.locator('.json-viewer').first()).toContainText('"theme": "warm"');

  await page.getByRole('button', { name: 'Edit object' }).click();
  await page.locator('.mutation-editor').fill('{');
  await expect(page.getByTestId('mutation-dialog').getByText(/Expected property name/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Submit request' })).toBeDisabled();
});

test('supports create, edit, delete, infer, and save workflows', async ({ page }) => {
  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await page.getByRole('button', { name: 'New row' }).click();
  await expect(page.getByTestId('mutation-dialog')).toBeVisible();
  await page.locator('.mutation-editor').fill(
    JSON.stringify(
      {
        name: 'Member 99',
        code: 'MEM-099',
        team_id: 1,
        city: 'Rome'
      },
      null,
      2
    )
  );
  await page.getByRole('button', { name: 'POST request' }).click();
  await expect(page.getByTestId('mutation-dialog')).toHaveCount(0);

  await page.getByRole('button', { name: 'Add filter' }).click();
  await page.locator('.filter-row select').first().selectOption('code');
  await page.locator('.filter-row input').fill('MEM-099');
  await expect(page.locator('tbody tr')).toHaveCount(1);
  await page.locator('tbody tr').first().click();

  await page.getByRole('button', { name: 'Edit row' }).click();
  await page.locator('.mutation-editor').fill(
    JSON.stringify(
      {
        id: 13,
        name: 'Member 99',
        code: 'MEM-099',
        team_id: 1,
        city: 'Milan'
      },
      null,
      2
    )
  );
  await page.getByRole('button', { name: 'PATCH request' }).click();
  await expect(page.getByTestId('inspector-panel')).toContainText('Milan');

  await page.getByRole('button', { name: 'Delete row' }).click();
  await page.getByLabel('I understand this delete cannot be undone.').check();
  await page.getByRole('button', { name: 'DELETE request' }).click();
  await expect(page.locator('tbody')).toContainText('No rows match the current query.');

  await page.getByRole('button', { name: 'Schema' }).click();
  await page.getByRole('button', { name: 'Infer from data' }).click();
  await expect(page.getByText(/Schema inferred/)).toBeVisible();
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Schema saved.')).toBeVisible();
});

async function waitForServer(baseUrl) {
  const deadline = Date.now() + 15_000;

  while (Date.now() < deadline) {
    try {
      const response = await fetch(baseUrl);
      if (response.ok) {
        return;
      }
    } catch {
      // Retry until the server is ready.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  throw new Error(`Server at ${baseUrl} did not start in time.`);
}
