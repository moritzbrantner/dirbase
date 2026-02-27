import { spawn } from 'node:child_process';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const __dirname = dirname(fileURLToPath(import.meta.url));

export function getBinaryPath(): string {
  return join(__dirname, '..', 'bin', process.platform === 'win32' ? 'folder-server.exe' : 'folder-server');
}

export function runFolderServer(args: string[]): Promise<number> {
  return new Promise((resolve, reject) => {
    const child = spawn(getBinaryPath(), args, { stdio: 'inherit' });

    child.on('error', reject);
    child.on('close', (code) => {
      resolve(code ?? 1);
    });
  });
}
