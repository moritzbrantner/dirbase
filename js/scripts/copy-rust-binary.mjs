import { chmod, copyFile, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const packageRoot = join(__dirname, '..');
const binaryName = process.platform === 'win32' ? 'folder-server.exe' : 'folder-server';
const source = join(packageRoot, '..', 'target', 'release', binaryName);
const outDir = join(packageRoot, 'bin');
const destination = join(outDir, binaryName);

await mkdir(outDir, { recursive: true });
await copyFile(source, destination);

if (process.platform !== 'win32') {
  await chmod(destination, 0o755);
}
