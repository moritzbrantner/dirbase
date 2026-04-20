import { chmod, copyFile, mkdir } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));
const packageRoot = join(__dirname, '..');
const platformKey = process.env.TARGET_PLATFORM_KEY ?? `${process.platform}-${process.arch}`;
const binaryName = process.env.TARGET_BINARY_NAME ?? (process.platform === 'win32' ? 'dirbase.exe' : 'dirbase');
const cargoTarget = process.env.CARGO_BUILD_TARGET ? join(process.env.CARGO_BUILD_TARGET, 'release') : 'release';
const source = join(packageRoot, '..', 'target', cargoTarget, binaryName);
const outDir = join(packageRoot, 'bin', platformKey);
const destination = join(outDir, binaryName);

await mkdir(outDir, { recursive: true });
await copyFile(source, destination);

if (process.platform !== 'win32') {
  await chmod(destination, 0o755);
}
