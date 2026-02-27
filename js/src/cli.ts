import { runFolderServer } from './index.js';

const code = await runFolderServer(process.argv.slice(2));
process.exit(code);
