import { runDirbase } from './index.js';

const code = await runDirbase(process.argv.slice(2));
process.exit(code);
