import { EventEmitter } from 'node:events';
import { join } from 'node:path';

import { describe, expect, mock, test } from 'bun:test';

import { binaryNameForPlatform, getBinaryPath, platformKey, runDirbase } from './index';

describe('binary path helpers', () => {
  test('builds platform keys from platform and architecture', () => {
    expect(platformKey('linux', 'x64')).toBe('linux-x64');
    expect(platformKey('darwin', 'arm64')).toBe('darwin-arm64');
  });

  test('selects executable names for Unix and Windows platforms', () => {
    expect(binaryNameForPlatform('linux')).toBe('dirbase');
    expect(binaryNameForPlatform('darwin')).toBe('dirbase');
    expect(binaryNameForPlatform('win32')).toBe('dirbase.exe');
  });

  test('resolves prebuilt binary paths by platform', () => {
    expect(getBinaryPath('linux', 'x64', '/pkg/dist')).toBe(
      join('/pkg/dist', '..', 'bin', 'linux-x64', 'dirbase')
    );
    expect(getBinaryPath('win32', 'x64', '/pkg/dist')).toBe(
      join('/pkg/dist', '..', 'bin', 'win32-x64', 'dirbase.exe')
    );
  });
});

describe('runDirbase', () => {
  test('rejects clearly when the platform binary is missing', async () => {
    await expect(
      runDirbase(['--help'], {
        binaryPath: '/missing/dirbase',
        exists: () => false
      })
    ).rejects.toThrow(`No prebuilt dirbase binary is available for ${platformKey()}.`);
  });

  test('spawns the resolved binary and resolves with its exit code', async () => {
    const child = new EventEmitter();
    const spawnProcess = mock((binaryPath: string, args: string[]) => {
      expect(binaryPath).toBe('/tmp/dirbase');
      expect(args).toEqual(['--version']);
      queueMicrotask(() => child.emit('close', 7));
      return child as ReturnType<typeof import('node:child_process').spawn>;
    });

    await expect(
      runDirbase(['--version'], {
        binaryPath: '/tmp/dirbase',
        exists: () => true,
        spawnProcess
      })
    ).resolves.toBe(7);
    expect(spawnProcess).toHaveBeenCalledTimes(1);
  });

  test('rejects when the spawned process emits an error', async () => {
    const child = new EventEmitter();
    const error = new Error('spawn failed');
    const spawnProcess = mock(() => {
      queueMicrotask(() => child.emit('error', error));
      return child as ReturnType<typeof import('node:child_process').spawn>;
    });

    await expect(
      runDirbase([], {
        binaryPath: '/tmp/dirbase',
        exists: () => true,
        spawnProcess
      })
    ).rejects.toThrow('spawn failed');
  });
});
