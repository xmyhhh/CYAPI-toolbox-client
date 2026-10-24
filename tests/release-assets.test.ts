import test from 'node:test';
import assert from 'node:assert/strict';
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { parse } from 'yaml';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { requiredReleaseAssets, verifyReleaseAssets } from '../scripts/release-assets.mjs';

test('website auto-deployment stays disabled while release publishing remains enabled', async () => {
  const source = await readFile(
    new URL('../.github/workflows/release.yml', import.meta.url),
    'utf8',
  );
  const workflow = parse(source);
  assert.equal(workflow.jobs.website.if, '${{ false }}');
  assert.match(source, /暂时不做官网自动部署/);
  assert.equal(workflow.jobs.website.needs, 'release');
  assert.ok(
    workflow.jobs.website.steps.some(
      (step: { name?: string }) => step.name === 'Deploy the website',
    ),
  );
  assert.ok(
    workflow.jobs.release.steps.some(
      (step: { name?: string; if?: string }) =>
        step.name === 'Create the release' && step.if === "env.DRY_RUN != 'true'",
    ),
  );
});

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
