import { build } from 'esbuild';

await build({
  entryPoints: ['src/main.tsx'],
  bundle: true,
  outfile: 'dist/overview.js',
  format: 'esm',
  platform: 'browser',
  target: 'es2022',
  sourcemap: false,
  minify: true,
  loader: {
    '.svg': 'dataurl'
  }
});
