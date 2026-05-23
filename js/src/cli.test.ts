import { describe, expect, mock, test } from 'bun:test';

import { main } from './cli';

describe('cli main', () => {
  test('exits with the resolved dirbase process code', async () => {
    const runDirbase = mock(async (args: string[]) => {
      expect(args).toEqual(['--version']);
      return 7;
    });
    const exit = mock((code: number) => {
      expect(code).toBe(7);
    });

    await main(['--version'], { runDirbase, exit });

    expect(runDirbase).toHaveBeenCalledTimes(1);
    expect(exit).toHaveBeenCalledTimes(1);
  });

  test('propagates runDirbase failures without forcing an exit code', async () => {
    const error = new Error('spawn failed');
    const runDirbase = mock(async () => {
      throw error;
    });
    const exit = mock(() => {});

    await expect(main([], { runDirbase, exit })).rejects.toThrow('spawn failed');
    expect(exit).not.toHaveBeenCalled();
  });
});
