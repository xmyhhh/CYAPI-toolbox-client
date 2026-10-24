import { stat } from 'node:fs/promises';
import { join, resolve } from 'node:path';
import { pathToFileURL } from 'node:url';

export function requiredReleaseAssets(version) {
  if (!/^\d+\.\d+\.\d+(?:-[\w.-]+)?$/.test(version)) throw new Error('Invalid release version');
  return [
    `CYAPI-Toolbox_${version}_x64-setup.exe`,
    `CYAPI-Toolbox_${version}.exe`,
    `CYAPI-Toolbox_${version}-macos-arm64.dmg`,
    `CYAPI-Toolbox_${version}-macos-x64.dmg`,
  ];
}

/** Fail closed: a successful ARM build must never hide a missing Intel installer. */
export async function verifyReleaseAssets(directory, version) {
  for (const file of requiredReleaseAssets(version)) {
    const info = await stat(join(directory, file)).catch(() => null);
    if (!info?.isFile() || info.size === 0)
      throw new Error(`Missing or empty release asset: ${file}`);
  }
}

if (process.argv[1] && import.meta.url === pathToFileURL(resolve(process.argv[1])).href) {
  const [directory, version] = process.argv.slice(2);
  if (!directory || !version)
    throw new Error('Usage: node scripts/release-assets.mjs <directory> <version>');
  await verifyReleaseAssets(directory, version);
  console.log(`Verified Windows, macOS ARM64 and Intel x64 installers for ${version}`);
}
