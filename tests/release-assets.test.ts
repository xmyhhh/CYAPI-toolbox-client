import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, rm, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { requiredReleaseAssets, verifyReleaseAssets } from '../scripts/release-assets.mjs';

test('a release requires both macOS architectures and both Windows packages', async () => {
  const directory = await mkdtemp(join(tmpdir(), 'cyapi-release-assets-'));
  try {
    const files = requiredReleaseAssets('1.2.1');
    assert.equal(files.length, 4);
    for (const file of files.filter((file) => !file.endsWith('-macos-x64.dmg')))
      await writeFile(join(directory, file), 'fixture');
    await assert.rejects(verifyReleaseAssets(directory, '1.2.1'), /macos-x64\.dmg/);
    await writeFile(join(directory, files[3]), '');
    await assert.rejects(verifyReleaseAssets(directory, '1.2.1'), /empty release asset/);
    await writeFile(join(directory, files[3]), 'fixture-intel');
    await verifyReleaseAssets(directory, '1.2.1');
    assert.throws(() => requiredReleaseAssets('../escape'), /Invalid release version/);
  } finally {
    await rm(directory, { recursive: true, force: true });
  }
});
