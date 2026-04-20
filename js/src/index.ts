import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));

type SpawnFn = typeof spawn;

interface RunDirbaseOptions {
  binaryPath?: string;
  exists?: (path: string) => boolean;
  spawnProcess?: SpawnFn;
}

export function platformKey(platform = process.platform, arch = process.arch): string {
  return `${platform}-${arch}`;
}

export function binaryNameForPlatform(platform = process.platform): string {
  return platform === 'win32' ? 'dirbase.exe' : 'dirbase';
}

export function getBinaryPath(
  platform = process.platform,
  arch = process.arch,
  baseDir = __dirname
): string {
  return join(baseDir, '..', 'bin', platformKey(platform, arch), binaryNameForPlatform(platform));
}

export function runDirbase(args: string[], options: RunDirbaseOptions = {}): Promise<number> {
  return new Promise((resolve, reject) => {
    const binaryPath = options.binaryPath ?? getBinaryPath();
    const exists = options.exists ?? existsSync;
    const spawnProcess = options.spawnProcess ?? spawn;
    if (!exists(binaryPath)) {
      reject(new Error(`No prebuilt dirbase binary is available for ${platformKey()}.`));
      return;
    }

    const child = spawnProcess(binaryPath, args, { stdio: 'inherit' });

    child.on('error', reject);
    child.on('close', (code) => {
      resolve(code ?? 1);
    });
  });
}
