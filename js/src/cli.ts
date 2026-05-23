import { runDirbase as defaultRunDirbase } from './index.js';

type CliDeps = {
  runDirbase?: (args: string[]) => Promise<number>;
  exit?: (code: number) => never | void;
};

export async function main(args: string[], deps: CliDeps = {}): Promise<void> {
  const runDirbase = deps.runDirbase ?? defaultRunDirbase;
  const exit = deps.exit ?? process.exit;
  const code = await runDirbase(args);
  exit(code);
}

if ((import.meta as ImportMeta & { main?: boolean }).main) {
  await main(process.argv.slice(2));
}
