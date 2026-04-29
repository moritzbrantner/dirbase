import { defineConfig } from 'vitest/config';

export default defineConfig({
  test: {
    environment: 'jsdom',
    globals: true,
    setupFiles: ['./src/setupTests.ts'],
    include: ['src/**/*.test.ts', 'src/**/*.test.tsx'],
    exclude: ['e2e/**', '**/node_modules/**'],
    pool: 'forks',
    coverage: {
      thresholds: {
        statements: 80,
        lines: 80,
        functions: 75,
        branches: 65
      }
    }
  }
});
