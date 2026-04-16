import { rm } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const packageRoot = join(__dirname, '..');

await Promise.all([
  rm(join(packageRoot, 'dist'), { force: true, recursive: true }),
  rm(join(packageRoot, 'bin'), { force: true, recursive: true })
]);
