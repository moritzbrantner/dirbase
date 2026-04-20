const { rm } = require('node:fs/promises');
const { join } = require('node:path');
const { expect, test } = require('@playwright/test');

const FIXTURE_SCHEMA_PATH = join(__dirname, 'fixtures', 'schema.json');

const BASE_SCHEMA = {
  tables: {
    members: {
      foreign_keys: {
        team_id: {
          target_table: 'teams',
          target_column: 'id'
        }
      }
    }
  }
};

test.describe('schema editor', () => {
  test.beforeEach(async ({ request }) => {
    const response = await request.put('/schema', {
      data: BASE_SCHEMA
    });

    expect(response.ok()).toBeTruthy();
  });

  test.afterAll(async () => {
    await rm(FIXTURE_SCHEMA_PATH, { force: true });
  });

  test('persists schema edits and reloads them from the server', async ({ page, request }) => {
    await page.goto('/?resource=members');
    await page.getByRole('button', { name: 'Schema' }).click();

    const editor = page.locator('.schema-editor');
    await expect(editor).toHaveValue(/"team_id"/);

    await editor.fill(`{
  "tables": {
    "members": {
      "foreign_keys": {
        "team_id": {
          "target_table": "teams",
          "target_column": "id"
        },
        "city": {
          "target_table": "teams",
          "target_column": "name"
        }
      }
    }
  }
}`);

    const saveResponsePromise = page.waitForResponse(
      (response) => response.url().endsWith('/schema') && response.request().method() === 'PUT'
    );
    await page.getByRole('button', { name: 'Save' }).click();
    const saveResponse = await saveResponsePromise;

    expect(saveResponse.ok()).toBeTruthy();
    await expect(page.getByText('Schema saved.')).toBeVisible();

    const schemaResponse = await request.get('/schema');
    expect(schemaResponse.ok()).toBeTruthy();
    const schemaPayload = await schemaResponse.json();
    expect(schemaPayload.tables.members.foreign_keys.city.target_column).toBe('name');

    await page.reload();
    await page.getByRole('button', { name: 'Schema' }).click();
    await expect(editor).toHaveValue(/"city"/);
    await expect(editor).toHaveValue(/"target_column": "name"/);
  });

  test('reloads the server schema and discards unsaved textarea changes', async ({ page }) => {
    await page.goto('/?resource=members');
    await page.getByRole('button', { name: 'Schema' }).click();

    const editor = page.locator('.schema-editor');
    await expect(editor).toHaveValue(/"team_id"/);

    await editor.fill(`{
  "tables": {
    "members": {
      "foreign_keys": {
        "city": {
          "target_table": "teams",
          "target_column": "name"
        }
      }
    }
  }
}`);
    await expect(editor).toHaveValue(/"city"/);

    await page.getByRole('button', { name: 'Reload' }).click();

    await expect(page.getByText('Schema reloaded from the server.')).toBeVisible();
    await expect(editor).toHaveValue(/"team_id"/);
    await expect(editor).not.toHaveValue(
      /"city":\s*\{\s*"target_table": "teams",\s*"target_column": "name"\s*\}/
    );
  });

  test('surfaces schema validation failures without overwriting the saved schema', async ({
    page,
    request
  }) => {
    await page.goto('/?resource=members');
    await page.getByRole('button', { name: 'Schema' }).click();

    const editor = page.locator('.schema-editor');
    await editor.fill(`{
  "tables": {
    "members": {
      "foreign_keys": {
        "team_id": {
          "target_table": "missing",
          "target_column": "id"
        }
      }
    }
  }
}`);

    const saveResponsePromise = page.waitForResponse(
      (response) => response.url().endsWith('/schema') && response.request().method() === 'PUT'
    );
    await page.getByRole('button', { name: 'Save' }).click();
    const saveResponse = await saveResponsePromise;

    expect(saveResponse.status()).toBe(400);
    await expect(page.getByTestId('inspector-panel').getByText(/targets unknown table 'missing'/)).toBeVisible();

    const schemaResponse = await request.get('/schema');
    expect(schemaResponse.ok()).toBeTruthy();
    const schemaPayload = await schemaResponse.json();
    expect(schemaPayload.tables.members.foreign_keys.team_id.target_table).toBe('teams');
  });
});
