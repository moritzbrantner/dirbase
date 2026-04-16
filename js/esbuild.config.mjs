import { build } from 'esbuild';

await Promise.all([
  build({
    entryPoints: ['src/index.ts'],
    outfile: 'dist/index.js',
    bundle: true,
    platform: 'node',
    format: 'esm',
    target: 'node20'
  }),
  build({
    entryPoints: ['src/cli.ts'],
    outfile: 'dist/cli.js',
    bundle: true,
    platform: 'node',
    format: 'esm',
    target: 'es2022',
    banner: {
      js: '#!/usr/bin/env bun'
    }
  })
]);
