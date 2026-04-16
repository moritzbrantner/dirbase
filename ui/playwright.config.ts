import { defineConfig } from '@playwright/test';

export default defineConfig({
  testDir: './e2e',
  timeout: 60_000,
  use: {
    baseURL: 'http://127.0.0.1:4510'
  },
  webServer: {
    command: 'cargo run -- --folder ./ui/e2e/fixtures --bind 127.0.0.1:4510',
    cwd: '..',
    url: 'http://127.0.0.1:4510',
    reuseExistingServer: true,
    timeout: 120_000
  }
});
