import test from 'node:test';
import assert from 'node:assert/strict';
import { parseSkillManifest } from '../src/skills/manifest.ts';
import { installationPath } from '../src/skills/targets.ts';
import { installer } from '../src/lib/api/installer.ts';
import { catalogCategories, filterCatalog } from '../src/lib/catalog.ts';
import { canManageInstallation, visibleInWorkspace } from '../src/lib/installations.ts';
import type { NativeRecord } from '../src/lib/api/native.ts';
import {
  integrationManifest,
  integrationComponent,
  integrationLabel,
} from '../src/features/integrations/model.ts';
import type { IntegrationDescriptor, IntegrationEntry } from '../src/lib/api/native.ts';

test('software tutorials accept older catalogs and never select a foreign platform binary', () => {
  const legacy = {
    schema: 2,
    integrations: [
      { id: 'example', actions: [{ id: 'search' }], component: { platform: 'windows-x64' } },
    ],
  };
  const manifest = integrationManifest(legacy);
  const descriptor = manifest.integrations[0];
  assert.deepEqual(descriptor.actions[0].fields, []);
  assert.deepEqual(descriptor.actions[0].args, {});
  assert.equal(integrationComponent(descriptor, 'macos-arm64'), undefined);
  assert.equal(integrationComponent(descriptor), undefined);
  assert.equal(integrationComponent(descriptor, 'windows-x64')?.platform, 'windows-x64');
  assert.deepEqual(
    integrationManifest({ schema: 3, integrations: [{ id: 'skill', skillId: 'example' }] })
      .integrations[0].actions,
    [],
  );
  assert.throws(() => integrationManifest({ schema: 5, integrations: [] }), /格式不受支持/);
  assert.equal('fields' in legacy.integrations[0].actions[0], false);
});

test('software status keeps installation separate from software and first-task verification', () => {
  const descriptor = { skillId: 'example' } as IntegrationDescriptor;
  const entry = {
    registered: true,
    health: { ready: true },
    readiness: { state: 'configured' },
  } as IntegrationEntry;
  assert.equal(integrationLabel(descriptor, entry), '待首次验证');
  assert.equal(
    integrationLabel(descriptor, { ...entry, readiness: { state: 'needs-attention', checks: [] } }),
    '需要处理',
  );
  assert.equal(integrationLabel(descriptor, { ...entry, registered: false }), '待准备');
  assert.equal(integrationLabel(descriptor), '待检查');
});

test('inherited Skills are visible without granting management from another workspace', () => {
  const record: NativeRecord = {
    id: 'claude-owned',
    skillId: 'test',
    title: 'Test',
    version: '1',
    revision: 'abc',
    target: 'claude',
    scope: 'global',
    path: '/home/.claude/skills/test',
    installedAt: 0,
    runtimes: [],
    packageSha256: '',
  };
  const inherited = {
    record,
    exists: true,
    managed: true,
    visibleTargets: ['claude', 'opencode'] as NativeRecord['target'][],
  };
  assert.equal(visibleInWorkspace(inherited, 'opencode'), true);
  assert.equal(visibleInWorkspace(inherited, 'codex'), false);
  assert.equal(canManageInstallation(inherited, 'opencode'), false);
  assert.equal(canManageInstallation(inherited, 'claude'), true);
  assert.equal(canManageInstallation({ ...inherited, managed: false }, 'claude'), false);
  assert.equal(visibleInWorkspace({ record, exists: true }, 'claude'), true);
  assert.equal(visibleInWorkspace({ record, exists: true }, 'opencode'), false);
});

const manifest =
  '---\nname: example-skill\ndescription: |\n  A useful skill.\n  With a longer description.\n---\n\n# Example';
const skill = parseSkillManifest(manifest);

test('category filtering combines multilingual search, preserves counts and accepts older catalogs', () => {
  const entries = [
    {
      skill: {
        id: 'react-guide',
        title: 'React Guide',
        description: '前端性能优化',
        category: '前端设计',
      },
    },
    { skill: { id: 'pdf', title: 'PDF', description: '文档转换', category: '文档知识' } },
    { skill: { id: 'legacy', title: 'Legacy', description: '旧版目录' } },
  ];
  assert.equal(filterCatalog(entries, ' ＲＥＡＣＴ 性能 ', '前端设计').length, 1);
  assert.equal(filterCatalog(entries, 'react', '文档知识').length, 0);
  assert.equal(filterCatalog(entries, '   ', '').length, 3);
  assert.equal(filterCatalog(entries, '', '其他')[0].skill.id, 'legacy');
  assert.equal(new Map(catalogCategories(entries)).get('前端设计'), 1);
  assert.equal(entries.length, 3);
});

test('reads standard YAML, multiline descriptions, BOM and CRLF', () => {
  const parsed = parseSkillManifest('\uFEFF' + manifest.replaceAll('\n', '\r\n'), 3);
  assert.equal(parsed.name, 'example-skill');
  assert.equal(parsed.description, 'A useful skill.\nWith a longer description.');
  assert.equal(parsed.fileCount, 3);
});

test('rejects invalid names, traversal, malformed YAML and missing fields', () => {
  for (const name of ['../escape', 'UPPERCASE', 'bad--name', '-start', 'a'.repeat(65)]) {
    assert.throws(() => parseSkillManifest(`---\nname: ${name}\ndescription: Test\n---\n`));
  }
  for (const input of [
    '# No metadata',
    '---\nname: valid-name\n---\n',
    '---\nname: valid\nname: duplicate\ndescription: test\n---\n',
  ])
    assert.throws(() => parseSkillManifest(input));
});

test('rejects oversized manifest before YAML parsing', () => {
  assert.throws(() => parseSkillManifest('x'.repeat(256 * 1024 + 1)), /256 KB/);
});

test('uses distinct global and project adapters for Codex, Claude and OpenCode', () => {
  assert.equal(
    installationPath(skill, { defaultTarget: 'codex', scope: 'global', projectPath: '' }),
    '~/.codex/skills/example-skill',
  );
  assert.equal(
    installationPath(skill, { defaultTarget: 'claude', scope: 'global', projectPath: '' }),
    '~/.claude/skills/example-skill',
  );
  assert.equal(
    installationPath(skill, { defaultTarget: 'opencode', scope: 'global', projectPath: '' }),
    '~/.config/opencode/skills/example-skill',
  );
  assert.equal(
    installationPath(skill, {
      defaultTarget: 'codex',
      scope: 'project',
      projectPath: 'C:\\Projects\\demo\\',
    }),
    'C:\\Projects\\demo\\.agents\\skills\\example-skill',
  );
  assert.equal(
    installationPath(skill, {
      defaultTarget: 'claude',
      scope: 'project',
      projectPath: '/work/demo/',
    }),
    '/work/demo/.claude/skills/example-skill',
  );
  assert.equal(
    installationPath(skill, {
      defaultTarget: 'opencode',
      scope: 'project',
      projectPath: 'C:\\Projects\\demo\\',
    }),
    'C:\\Projects\\demo\\.opencode\\skills\\example-skill',
  );
});

test('requires an absolute, unambiguous project directory', () => {
  for (const projectPath of ['', 'relative/path', 'C:\\work\\..\\escape', '/work/./demo']) {
    assert.throws(() =>
      installationPath(skill, { defaultTarget: 'codex', scope: 'project', projectPath }),
    );
  }
});

test('simulates progress and requires explicit conflict replacement', async () => {
  const settings = { defaultTarget: 'codex', scope: 'global', projectPath: '' } as const;
  const plan = installer.preview(skill, settings, []);
  const progress: number[] = [];
  const record = await installer.install(plan, false, (step) => progress.push(step));
  assert.equal(installer.mode, 'demo');
  assert.deepEqual(progress, [0, 1, 2, 3, 4]);
  assert.equal(record.path, '~/.codex/skills/example-skill');
  const conflict = installer.preview(skill, settings, [record]);
  assert.equal(conflict.conflict?.id, record.id);
  await assert.rejects(
    installer.install(conflict, false, () => {}),
    /确认替换/,
  );
  assert.equal(
    installer.preview(skill, { ...settings, defaultTarget: 'claude' }, [record]).conflict,
    undefined,
  );
});
