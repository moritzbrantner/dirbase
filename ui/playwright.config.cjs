const { defineConfig } = require('@playwright/test');

module.exports = defineConfig({
  testDir: './e2e',
  testMatch: '**/*.spec.cjs',
  timeout: 60_000,
  use: {
    baseURL: 'http://127.0.0.1:4510'
  },
  webServer: {
    command:
      `bash -lc 'cd ui && npm run build && cd .. && tmpdir=$(mktemp -d) && cp ./ui/e2e/fixtures/* "$tmpdir"/ && cargo run -- --folder "$tmpdir" --bind 127.0.0.1:4510'`,
    cwd: '..',
    url: 'http://127.0.0.1:4510',
    reuseExistingServer: true,
    timeout: 120_000
  }
});
