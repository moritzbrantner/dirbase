import { expect, test } from '@playwright/test';

test('mobile surface controls remain operable above open drawers and sheets', async ({ page }) => {
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto('/?resource=members');
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });

  const surfaceControls = page.locator('.mobile-sticky-actions');
  await expect(surfaceControls).toBeVisible();

  await page.getByRole('button', { name: 'Resources', exact: true }).click();
  const resourceDrawer = page.getByTestId('resource-sidebar');
  await expect(resourceDrawer).toHaveClass(/mobile-drawer-open/);
  await expect(surfaceControls).toBeVisible();

  const controlZIndex = await surfaceControls.evaluate((element) =>
    Number.parseInt(window.getComputedStyle(element).zIndex || '0', 10)
  );
  const drawerZIndex = await resourceDrawer.evaluate((element) =>
    Number.parseInt(window.getComputedStyle(element).zIndex || '0', 10)
  );
  expect(controlZIndex).toBeGreaterThan(drawerZIndex);

  await page.getByRole('button', { name: 'Resources', exact: true }).click();
  await expect(resourceDrawer).not.toHaveClass(/mobile-drawer-open/);

  await page.locator('.result-card').first().click();
  const inspector = page.getByTestId('inspector-panel');
  await expect(inspector).toHaveClass(/mobile-sheet-open/);
  await expect(surfaceControls).toBeVisible();

  await page.getByRole('button', { name: 'Inspector', exact: true }).click();
  await expect(inspector).not.toHaveClass(/mobile-sheet-open/);
});
