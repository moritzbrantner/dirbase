import { expect, test } from '@playwright/test';

test('overview explorer supports selection, sorting, filtering, pagination, graph clicks, and relation drill-down', async ({
  page
}) => {
  await page.goto('/?resource=members');

  await expect(page.getByRole('heading', { name: 'members' })).toBeVisible();
  await expect(page.locator('.request-path')).toContainText('/members?page=1&per_page=25');

  await page.locator('.data-table thead button').filter({ hasText: 'name' }).click();
  await page.waitForTimeout(300);
  await expect(page.locator('.request-path')).toContainText('sort=name');

  await page.getByRole('button', { name: 'Add filter' }).click();
  await page.locator('.filter-row select').first().selectOption('code');
  await page.locator('.filter-row input').fill('MEM-012');
  await page.waitForTimeout(800);
  await expect(page.locator('tbody tr')).toHaveCount(1);
  await expect(page.locator('.request-path')).toContainText('code=MEM-012');

  await page.locator('.filter-row input').fill('');
  await page.waitForTimeout(800);
  await page.getByRole('combobox', { name: 'Page size' }).selectOption('10');
  await page.waitForTimeout(300);
  await page.getByRole('button', { name: /^Next$/ }).click();
  await page.waitForTimeout(300);
  await expect(page.locator('.request-path')).toContainText('page=2');
  await expect(page.locator('.request-path')).toContainText('per_page=10');

  await page.locator('.react-flow__node').filter({ hasText: 'teams' }).first().click();
  await expect(page.getByRole('heading', { name: 'teams' })).toBeVisible();

  await page.getByRole('button', { name: /members table/i }).click();
  await expect(page.getByRole('heading', { name: 'members' })).toBeVisible();
  await page.locator('tbody tr').first().click();
  await page.locator('.relation-link-button').filter({ hasText: 'teams' }).first().click();
  await expect(page.getByRole('heading', { name: 'teams' })).toBeVisible();
  await expect(page.locator('.request-path')).toContainText('/teams?page=1&per_page=25&id=');
});
