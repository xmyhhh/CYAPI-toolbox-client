// Runs one of the platform helper scripts in `scripts/`.
//
// The Windows helpers are PowerShell scripts, the macOS and Linux helpers are
// POSIX shell scripts. This dispatcher keeps the npm scripts identical on
// every platform.
import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';
import path from 'node:path';

const helpers = path.dirname(fileURLToPath(import.meta.url));
const isWindows = process.platform === 'win32';

/** Translate the shared flags into the form the target script expects. */
function translate(args) {
  return args.map((value) => {
    if (value === '--portable-only') return isWindows ? '-PortableOnly' : '--portable-only';
    if (value === '--use-mirror') return isWindows ? '-UseMirror' : '--use-mirror';
    return value;
  });
}

export function run(base, args = []) {
  const target = path.join(helpers, isWindows ? `${base}.ps1` : `${base}.sh`);
  const command = isWindows
    ? ['powershell', '-NoProfile', '-ExecutionPolicy', 'Bypass', '-File', target]
    : ['sh', target];
  const result = spawnSync(command[0], [...command.slice(1), ...translate(args)], {
    stdio: 'inherit',
  });
  if (result.error) {
    console.error(result.error.message);
    process.exit(1);
  }
  process.exit(result.status ?? 1);
}

if (process.argv[1] && path.resolve(process.argv[1]) === path.resolve(fileURLToPath(import.meta.url))) {
  const [base, ...args] = process.argv.slice(2);
  if (!base) {
    console.error('Usage: node scripts/run.mjs <script> [args...]');
    process.exit(1);
  }
  run(base, args);
}
