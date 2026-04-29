import { expect, test } from '@playwright/test';

test.describe('schema workspace', () => {
  test('loads dedicated schema mode and stages structured edits before save', async ({
    page,
    request
  }) => {
    await page.goto('/?resource=members&mode=schema');

    await expect(page.getByText('Graph-first schema editing')).toBeVisible();
    await expect(page.getByRole('button', { name: 'Schema' })).toHaveClass(/is-active/);
    await expect(page.locator('.schema-details h3').filter({ hasText: 'members' })).toBeVisible();

    await page.locator('.schema-details .overview-select').first().selectOption('relation');

    const unsavedEditor = await request.get('/schema/editor');
    const unsavedPayload = await unsavedEditor.json();
    expect(unsavedPayload.declared.tables.members.kind).toBe('object');

    await page.getByRole('button', { name: 'Save' }).click();
    await expect(page.getByText('Schema saved.')).toBeVisible();
    await expect
      .poll(async () => {
        const savedSchema = await request.get('/schema');
        const savedPayload = await savedSchema.json();
        return savedPayload.tables.members.kind;
      })
      .toBe('relation');
  });

  test('removes inferred relations by persisting suppression state', async ({ page, request }) => {
    await page.goto('/?resource=members&mode=schema');

    await page.getByRole('button', { name: /team_id teams.id/ }).click();
    await expect(page.getByText('Origin: manual')).toBeVisible();
    await page.getByRole('button', { name: 'Reset to inferred' }).click();
    await expect(page.getByText('Origin: inferred')).toBeVisible();

    await page.getByRole('button', { name: 'Remove relation' }).click();
    await page.getByRole('button', { name: 'Save' }).click();
    await expect(page.getByText('Schema saved.')).toBeVisible();

    const editorSchema = await request.get('/schema/editor');
    const editorPayload = await editorSchema.json();
    expect(editorPayload.declared.tables.members.suppressed_foreign_keys).toEqual(['team_id']);

    const effectiveSchema = await request.get('/schema');
    const effectivePayload = await effectiveSchema.json();
    expect(effectivePayload.tables.members.foreign_keys.team_id).toBeUndefined();

    await page.reload();
    await page.goto('/?resource=members&mode=schema');
    await expect(page.getByRole('button', { name: /team_id teams.id/ })).toHaveCount(0);
  });

  test('invalid JSON disables save until the draft is fixed or discarded', async ({ page }) => {
    await page.goto('/?resource=members&mode=schema');

    await page.getByRole('button', { name: 'JSON' }).click();
    const editor = page.locator('.schema-json-editor');
    await editor.fill('{');

    await expect(
      page.getByTestId('schema-json-drawer').getByText(/JSON Parse error|Expected property name|Expected/)
    ).toBeVisible();
    await expect(page.getByRole('button', { name: 'Save' })).toBeDisabled();

    await page.getByRole('button', { name: 'Discard invalid JSON changes' }).click();
    await expect(editor).toHaveValue(/"tables"/);
  });
});
