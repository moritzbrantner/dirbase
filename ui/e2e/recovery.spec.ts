import { expect, test } from '@playwright/test';
import { spawn, type ChildProcess } from 'node:child_process';
import { cpSync, mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

test.describe.configure({ mode: 'serial' });

const port = 4512;
const baseUrl = `http://127.0.0.1:${port}`;
let server: ChildProcess | null = null;
let serverFolder = '';

test.beforeAll(async () => {
  const fixtureCopy = mkdtempSync(join(tmpdir(), 'dirbase-overview-recovery-'));
  serverFolder = join(fixtureCopy, 'fixtures');
  cpSync(join(process.cwd(), 'e2e/fixtures'), serverFolder, { recursive: true });
  server = await startServer();
});

test.afterAll(async () => {
  await stopServer();
});

test('recovers after the overview server is restarted', async ({ page }) => {
  await page.goto(`${baseUrl}/?resource=members`);
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });
  await expect(page.locator('.status-pill').filter({ hasText: 'Live Live' })).toBeVisible({
    timeout: 15_000
  });

  await stopServer();
  await expect(page.locator('.status-pill').filter({ hasText: 'Live Paused' })).toBeVisible({
    timeout: 20_000
  });

  server = await startServer();
  await page.getByRole('button', { name: 'Retry live updates' }).click();
  await expect(page.locator('.status-pill').filter({ hasText: 'Live Live' })).toBeVisible({
    timeout: 20_000
  });
  await page.reload();
  await page.locator('.resource-list-item').first().waitFor({ state: 'visible', timeout: 15_000 });
  await expect(page.getByTestId('resource-sidebar')).toContainText('members');
});

async function startServer(): Promise<ChildProcess> {
  const child = spawn(
    'cargo',
    ['run', '--', '--folder', serverFolder, '--bind', `127.0.0.1:${port}`],
    {
      cwd: '..',
      stdio: 'pipe'
    }
  );
  await waitForServer(baseUrl);
  return child;
}

async function stopServer() {
  if (!server) {
    return;
  }

  const current = server;
  server = null;
  current.kill('SIGTERM');
  await new Promise<void>((resolve) => {
    current.once('exit', () => resolve());
    setTimeout(resolve, 2_000);
  });
}

async function waitForServer(url: string) {
  const deadline = Date.now() + 30_000;

  while (Date.now() < deadline) {
    try {
      const response = await fetch(url);
      if (response.ok) {
        return;
      }
    } catch {
      // Retry until the server is ready.
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }

  throw new Error(`Server at ${url} did not start in time.`);
}
