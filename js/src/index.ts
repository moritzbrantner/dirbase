import { spawn } from 'node:child_process';
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));

function platformKey(): string {
  return `${process.platform}-${process.arch}`;
}

export function getBinaryPath(): string {
  const binaryName = process.platform === 'win32' ? 'folder-server.exe' : 'folder-server';
  return join(__dirname, '..', 'bin', platformKey(), binaryName);
}

export function runFolderServer(args: string[]): Promise<number> {
  return new Promise((resolve, reject) => {
    const binaryPath = getBinaryPath();
    if (!existsSync(binaryPath)) {
      reject(new Error(`No prebuilt folder-server binary is available for ${platformKey()}.`));
      return;
    }

    const child = spawn(binaryPath, args, { stdio: 'inherit' });

    child.on('error', reject);
    child.on('close', (code) => {
      resolve(code ?? 1);
    });
  });
}
