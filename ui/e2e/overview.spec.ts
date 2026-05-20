import { expect, test, type Page } from '@playwright/test';
import { spawn, type ChildProcess } from 'node:child_process';
import { cpSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test.describe.configure({ mode: 'serial' });

let readonlyServer: ChildProcess | null = null;

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
  await expect(page.getByTestId('query-summary')).toHaveCount(0);

  await page.locator('.result-card').first().click();
  await expect(page.getByTestId('inspector-panel')).toContainText('Selected row');
  await page.locator('.relation-link-button').filter({ hasText: 'teams' }).first().click();
  await expect(page.locator('.workspace-main').getByRole('heading', { name: 'teams' })).toBeVisible();
  await expect(page.locator('.request-path').first()).toContainText('/teams?page=1&per_page=25&id=1');

  await page.locator('.result-card').first().click();
  await page.locator('.relation-link-button').filter({ hasText: 'members' }).first().click();
  await expect(page.locator('.workspace-main').getByRole('heading', { name: 'members' })).toBeVisible();
  await expect(page.locator('.request-path').first()).toContainText('team_id=1');
});

test('supports mobile drawers and inspector sheet without losing explorer state', async ({
  page
}) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await page.getByRole('button', { name: 'Resources' }).click();
  await expect(page.getByTestId('resource-sidebar')).toHaveClass(/mobile-drawer-open/);

  await page.getByRole('button', { name: 'Map', exact: true }).click();
  await expect(page.locator('.relation-map-panel')).toHaveClass(/mobile-drawer-open/);

  await page.getByRole('button', { name: 'Map', exact: true }).click();
  await page.locator('.result-card').first().click();
  await expect(page.getByTestId('inspector-panel')).toHaveClass(/mobile-sheet-open/);
  await expect(page.locator('.workspace-main').getByRole('heading', { name: 'members' })).toBeVisible();
});

test('opens the relation map as a secondary desktop panel', async ({ page }) => {
  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await page.getByRole('button', { name: 'Open map' }).click();
  await expect(page.locator('.relation-map-panel')).toHaveClass(/mobile-drawer-open/);
  await page.getByRole('button', { name: 'Close map' }).first().click();
  await expect(page.locator('.relation-map-panel')).not.toHaveClass(/mobile-drawer-open/);
});

test('hides write actions in read-only mode', async ({ page }) => {
  await page.goto('http://127.0.0.1:4511/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await expect(page.locator('.status-pill.is-warn').first()).toBeVisible();
  await expect(page.getByRole('button', { name: 'New row' })).toHaveCount(0);
  await expect(page.getByRole('button', { name: 'Delete row' })).toHaveCount(0);

  await page.getByRole('button', { name: 'Schema' }).click();
  await expect(page.getByRole('button', { name: 'Infer from data' })).toBeDisabled();
  await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled();
});

test('supports object raw mode and invalid mutation feedback', async ({ page }) => {
  await page.goto('/?resource=settings');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await expect(page.locator('.workspace-main').getByRole('heading', { name: 'settings' })).toBeVisible();
  await page.getByRole('button', { name: 'Raw JSON' }).click();
  await expect(page.locator('.json-viewer').first()).toContainText('"theme": "warm"');

  await page.getByRole('button', { name: 'Edit object' }).click();
  await page.locator('.mutation-editor').fill('{');
  await expect(page.getByTestId('mutation-dialog').getByText(/Expected property name/)).toBeVisible();
  await expect(page.getByRole('button', { name: 'Stage change' })).toBeDisabled();
});

test('creates a table row through the UI and shows it in filtered results', async ({
  page,
  request
}) => {
  const createdMember = {
    name: 'Playwright Created Member',
    code: `MEM-E2E-${Date.now()}`,
    team_id: 2,
    city: 'Vienna'
  };

  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await page.getByRole('button', { name: 'New row' }).click();
  await expect(page.getByTestId('mutation-dialog')).toBeVisible();
  await page.locator('.mutation-editor').fill(JSON.stringify(createdMember, null, 2));
  await expect(page.locator('.mutation-plan-card .request-path')).toContainText('POST /members');
  await page.getByRole('button', { name: 'Stage change' }).click();
  await expect(page.getByTestId('mutation-dialog')).toHaveCount(0);
  await expect(page.getByTestId('staged-mutations')).toContainText('POST /members');

  await filterResultsBy(page, 'code', createdMember.code);
  await expect(page.locator('.result-shell')).toContainText('No rows match the current query.');

  await page.getByRole('button', { name: 'Save changes' }).click();
  await expect(page.getByTestId('staged-mutations')).toHaveCount(0);
  await expect(page.locator('.result-card')).toHaveCount(1);
  await expect(page.locator('.result-card').first()).toContainText(createdMember.name);
  await expect(page.locator('.result-card').first()).toContainText(createdMember.city);

  const response = await request.get(`/members?code=${encodeURIComponent(createdMember.code)}`);
  expect(response.ok()).toBeTruthy();
  expect(collectionRows(await response.json())).toContainEqual(expect.objectContaining(createdMember));
});

test('edits a selected table row through the UI and persists the changed fields', async ({
  page,
  request
}) => {
  const editedMember = {
    id: 2,
    name: 'Member 02 Edited',
    code: 'MEM-002',
    team_id: 2,
    city: 'Salzburg'
  };

  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await filterResultsBy(page, 'code', editedMember.code);
  await expect(page.locator('.result-card')).toHaveCount(1);
  await page.locator('.result-card').first().click();

  await page.getByRole('button', { name: 'Edit row' }).click();
  await expect(page.getByTestId('mutation-dialog')).toBeVisible();
  await page.locator('.mutation-editor').fill(JSON.stringify(editedMember, null, 2));
  await expect(page.locator('.mutation-plan-card .request-path')).toContainText('PATCH /members/2');
  await expect(page.locator('.mutation-plan-card')).toContainText('Changed keys: name, city');
  await page.getByRole('button', { name: 'Stage change' }).click();
  await expect(page.getByTestId('mutation-dialog')).toHaveCount(0);
  await expect(page.getByTestId('staged-mutations')).toContainText('PATCH /members/2');

  const unchangedResponse = await request.get('/members/2');
  expect(unchangedResponse.ok()).toBeTruthy();
  expect(await unchangedResponse.json()).toEqual(
    expect.objectContaining({ id: 2, name: 'Member 02', city: 'Munich' })
  );

  await page.getByRole('button', { name: 'Save changes' }).click();
  await expect(page.getByTestId('staged-mutations')).toHaveCount(0);
  await page.locator('.result-card').first().click();
  await expect(page.getByTestId('inspector-panel')).toContainText(editedMember.name);
  await expect(page.getByTestId('inspector-panel')).toContainText(editedMember.city);
  await expect(page.locator('.result-card').first()).toContainText(editedMember.name);

  const response = await request.get('/members/2');
  expect(response.ok()).toBeTruthy();
  expect(await response.json()).toEqual(expect.objectContaining(editedMember));
});

test('blocks saving staged changes that break schema relationships', async ({ page, request }) => {
  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  await filterResultsBy(page, 'code', 'MEM-003');
  await expect(page.locator('.result-card')).toHaveCount(1);
  await page.locator('.result-card').first().click();

  await page.getByRole('button', { name: 'Edit row' }).click();
  await page.locator('.mutation-editor').fill(
    JSON.stringify(
      {
        id: 3,
        name: 'Member 03',
        code: 'MEM-003',
        team_id: 999,
        city: 'Hamburg'
      },
      null,
      2
    )
  );
  await page.getByRole('button', { name: 'Stage change' }).click();
  await page.getByRole('button', { name: 'Save changes' }).click();

  await expect(page.getByTestId('staged-mutations')).toContainText('Save blocked by broken references');
  await expect(page.getByTestId('staged-mutations')).toContainText(
    'members.team_id value 999 does not reference an existing teams.id'
  );

  const response = await request.get('/members/3');
  expect(response.ok()).toBeTruthy();
  expect(await response.json()).toEqual(expect.objectContaining({ id: 3, team_id: 3 }));

  await page.getByRole('button', { name: 'Discard changes' }).click();
  await expect(page.getByTestId('staged-mutations')).toHaveCount(0);
});

test('supports create, edit, delete, infer, and save workflows', async ({ page, request }) => {
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
  await page.getByRole('button', { name: 'Stage change' }).click();
  await expect(page.getByTestId('mutation-dialog')).toHaveCount(0);
  await page.getByRole('button', { name: 'Save changes' }).click();
  await expect(page.getByTestId('staged-mutations')).toHaveCount(0);

  await page.getByRole('button', { name: 'Add filter' }).click();
  await page.locator('.filter-row select').first().selectOption('code');
  await page.locator('.filter-row input').fill('MEM-099');
  await expect(page.locator('.result-card')).toHaveCount(1);
  await page.locator('.result-card').first().click();

  const createdRowsResponse = await request.get('/members?code=MEM-099');
  expect(createdRowsResponse.ok()).toBeTruthy();
  const createdRows = collectionRows(await createdRowsResponse.json()) as Array<Record<string, unknown>>;
  const createdMemberId = createdRows[0]?.id;
  expect(createdMemberId).toBeDefined();

  await page.getByRole('button', { name: 'Edit row' }).click();
  await page.locator('.mutation-editor').fill(
    JSON.stringify(
      {
        id: createdMemberId,
        name: 'Member 99',
        code: 'MEM-099',
        team_id: 1,
        city: 'Milan'
      },
      null,
      2
    )
  );
  await page.getByRole('button', { name: 'Stage change' }).click();
  await page.getByRole('button', { name: 'Save changes' }).click();
  await expect(page.getByTestId('staged-mutations')).toHaveCount(0);
  await expect(page.locator('.result-card').first()).toContainText('Milan');
  await page.locator('.result-card').first().click();

  await page.getByRole('button', { name: 'Delete row' }).click();
  await page.getByLabel('I understand this delete cannot be undone.').check();
  await page.getByRole('button', { name: 'Stage change' }).click();
  await page.getByRole('button', { name: 'Save changes' }).click();
  await expect(page.getByTestId('staged-mutations')).toHaveCount(0);
  await expect(page.locator('.result-shell')).toContainText('No rows match the current query.');

  await page.getByRole('button', { name: 'Schema' }).click();
  await page.getByRole('button', { name: 'Infer from data' }).click();
  await expect(page.getByText(/Schema inferred/)).toBeVisible();
  await page.getByRole('button', { name: 'Save' }).click();
  await expect(page.getByText('Schema saved.')).toBeVisible();
});

async function waitForServer(baseUrl: string) {
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

async function filterResultsBy(page: Page, field: string, value: string) {
  await page.getByRole('button', { name: 'Add filter' }).click();
  const filterRow = page.locator('.filter-row').last();
  await filterRow.locator('select').first().selectOption(field);
  await filterRow.locator('input').fill(value);
  await expect(page.getByTestId('query-summary')).toContainText(`${field}`);
  await expect(page.getByTestId('query-summary')).toContainText(`equals ${value}`);
}

function collectionRows(payload: unknown): unknown[] {
  if (Array.isArray(payload)) {
    return payload;
  }

  if (payload && typeof payload === 'object' && Array.isArray((payload as { data?: unknown }).data)) {
    return (payload as { data: unknown[] }).data;
  }

  return [];
}
