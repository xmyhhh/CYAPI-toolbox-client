import test from 'node:test';
import assert from 'node:assert/strict';
import { parseSkillManifest } from '../src/skills/manifest.ts';
import { installationPath } from '../src/skills/targets.ts';
import { installer } from '../src/lib/api/installer.ts';
import { catalogCategories, filterCatalog } from '../src/lib/catalog.ts';
import { canManageInstallation, visibleInWorkspace } from '../src/lib/installations.ts';
import type { NativeRecord } from '../src/lib/api/native.ts';
import type { CodexConfiguration } from '../src/lib/api/native.ts';
import {
  CodexSaveConflict,
  codexSaveConflictMessage,
  prepareCodexSave,
} from '../src/lib/codex-save.ts';
import {
  configuredModels,
  modelEffort,
  claudeModelEffort,
  changeModelEffort,
  selectedEffortChanges,
  contextWindowText,
  parseContextWindow,
  contextWindowChange,
  selectedContextChanges,
  reasoningLevelsText,
  selectedReasoningLevelChanges,
} from '../src/lib/model-settings.ts';
import {
  integrationManifest,
  integrationComponent,
  integrationLabel,
} from '../src/features/integrations/model.ts';
import type { IntegrationDescriptor, IntegrationEntry } from '../src/lib/api/native.ts';

test('model selection keeps a single default in the configured set without duplicates', () => {
  assert.deepEqual(configuredModels([' a ', 'b', 'a', ''], 'b'), ['a', 'b']);
  assert.deepEqual(configuredModels(['a'], 'c'), ['a', 'c']);
  assert.deepEqual(configuredModels([], ''), []);
});

test('Codex save rebases disjoint model edits without discarding externally repaired metadata', () => {
  const before: CodexConfiguration = {
    configPath: '/home/.codex/config.toml',
    authPath: '/home/.codex/auth.json',
    revision: 'old',
    saveRevision: 'old',
    rebaseGuard: 'same-provider-auth',
    provider: 'cyapi-toolbox',
    baseUrl: 'https://example.invalid/v1',
    model: 'sol',
    models: ['sol'],
    reasoningEffort: 'none',
    modelSettings: { sol: { reasoningLevels: [], defaultLevel: null } },
    apiKeyConfigured: true,
    authMode: 'API Key',
    canRestore: false,
  };
  const latest: CodexConfiguration = {
    ...before,
    revision: 'new',
    saveRevision: 'new',
    reasoningEffort: '',
    modelSettings: {
      sol: { reasoningLevels: ['low', 'medium', 'high', 'xhigh', 'max'], defaultLevel: 'medium' },
    },
  };
  const draft = {
    baseUrl: before.baseUrl,
    apiKey: '',
    model: 'sol',
    models: ['sol'],
    reasoningEffort: 'none',
    modelSettings: { sol: { reasoningLevels: null, defaultLevel: null, contextWindow: 100000 } },
  };
  const merged = prepareCodexSave(before, latest, draft);
  assert.equal(merged.reasoningEffort, '');
  assert.deepEqual(merged.modelSettings.sol, {
    reasoningLevels: ['low', 'medium', 'high', 'xhigh', 'max'],
    defaultLevel: 'medium',
    contextWindow: 100000,
  });
  assert.equal(merged.revision, 'new');
  assert.equal(merged.saveRevision, 'new');
  assert.equal(latest.modelSettings?.sol.contextWindow, undefined);
  assert.throws(
    () => prepareCodexSave(before, latest, { ...draft, reasoningEffort: 'max' }),
    CodexSaveConflict,
  );
  assert.throws(
    () => prepareCodexSave(before, { ...latest, rebaseGuard: 'new-auth' }, draft),
    CodexSaveConflict,
  );
  assert.throws(
    () => prepareCodexSave(before, { ...latest, configPath: '/another/config.toml' }, draft),
    CodexSaveConflict,
  );
  assert.throws(
    () => prepareCodexSave({ ...before, rebaseGuard: undefined }, latest, draft),
    CodexSaveConflict,
  );
  assert.equal(
    prepareCodexSave(before, { ...before, revision: 'background-comment-change' }, draft).revision,
    'background-comment-change',
  );
  assert.equal(
    prepareCodexSave(before, latest, { ...draft, reasoningEffort: '' }).reasoningEffort,
    '',
  );
});

test('only recognized Codex save conflicts avoid the automatic error-log dialog', () => {
  const text =
    'Codex 的模型、线路或认证设置已被其他程序修改，请重新读取并核对后保存；当前输入已保留';
  assert.ok(codexSaveConflictMessage('codex-save', text));
  assert.ok(codexSaveConflictMessage('codex-save', 'Codex 配置已被其他程序修改，请重新读取'));
  assert.equal(codexSaveConflictMessage('codex-restore', text), undefined);
  assert.equal(
    codexSaveConflictMessage('codex-save', '无法替换 Codex 配置文件，请检查是否被占用'),
    undefined,
  );
  assert.equal(
    codexSaveConflictMessage(
      'codex-save',
      'Codex 配置保存中断，请用本工具的备份恢复，或检查配置文件',
    ),
    undefined,
  );
});

test('OpenCode menu levels are explicit capability patches, separate from request defaults', () => {
  assert.equal(reasoningLevelsText(undefined), '');
  assert.equal(reasoningLevelsText(['low', 'xhigh', 'max']), 'low, xhigh, max');
  assert.equal(reasoningLevelsText({ future: true }), '{"future":true}');
  assert.deepEqual(selectedReasoningLevelChanges({}, ['sol'], 'sol'), {});
  assert.deepEqual(
    selectedReasoningLevelChanges(
      { sol: 'low, medium, high, xhigh, max', other: 'bad value' },
      [],
      'sol',
    ),
    {
      sol: ['low', 'medium', 'high', 'xhigh', 'max'],
    },
  );
  assert.deepEqual(selectedReasoningLevelChanges({ sol: '' }, ['sol'], ''), { sol: null });
  assert.deepEqual(selectedReasoningLevelChanges({ sol: 'custom-future' }, ['sol'], ''), {
    sol: ['custom-future'],
  });
  for (const text of ['low,low', 'low,', 'bad value', 'a'.repeat(65), 'low,hi\u0000']) {
    assert.throws(() => selectedReasoningLevelChanges({ sol: text }, [], 'sol'));
  }
});

test('new full Claude IDs reuse existing canonical settings without guessing aliases or gateway IDs', () => {
  const efforts = { 'claude-sonnet-4-6': 'high', 'gateway-20260101': 'low' };
  const added = 'claude-sonnet-4-6-20260101';
  assert.equal(claudeModelEffort(added, efforts), 'high');
  assert.equal(claudeModelEffort(`${added}[1m]`, efforts), 'high');
  assert.deepEqual(changeModelEffort({}, added, '', claudeModelEffort(added, efforts)), {
    [added]: '',
  });
  assert.equal(claudeModelEffort(added, { ...efforts, [added]: 'low' }), 'low');
  assert.equal(claudeModelEffort('claude-sonnet-4-6', { [added]: 'medium' }), 'medium');
  assert.equal(
    claudeModelEffort('claude-sonnet-4-6', {
      [added]: 'medium',
      'claude-sonnet-4-6-20260201': 'low',
    }),
    '',
  );
  assert.equal(claudeModelEffort('sonnet', efforts), '');
  assert.equal(claudeModelEffort('sonnet', efforts, { sonnet: 'claude-sonnet-4-6' }), 'high');
  assert.equal(claudeModelEffort('gateway', efforts), '');
  assert.equal(claudeModelEffort('constructor', {}), '');
});

test('per-model effort patches preserve untouched settings and distinguish clear from preserve', () => {
  const efforts = { a: 'high', b: 'low', canonical: 'future-level' };
  assert.equal(modelEffort('alias', efforts, { alias: 'canonical' }), 'future-level');
  assert.equal(modelEffort('new', efforts), '');
  const changed = changeModelEffort({}, 'a', 'xhigh', efforts.a);
  assert.deepEqual(changed, { a: 'xhigh' });
  assert.deepEqual(efforts, { a: 'high', b: 'low', canonical: 'future-level' });
  assert.deepEqual(changeModelEffort(changed, 'a', 'high', efforts.a), {});
  assert.deepEqual(changeModelEffort({}, 'a', '', efforts.a), { a: '' });
  assert.deepEqual(changeModelEffort({}, 'new', '', ''), {});
  // Conflicting OpenCode aliases have no common original; clearing must still be explicit.
  assert.deepEqual(changeModelEffort({}, 'a', '', '', true), { a: '' });
  assert.deepEqual(changeModelEffort({}, 'a', 'high', '', true), { a: 'high' });
  assert.equal(modelEffort('constructor', {}), '');
  assert.deepEqual(Object.entries(changeModelEffort({}, '__proto__', 'high', '')), [
    ['__proto__', 'high'],
  ]);
  assert.deepEqual(selectedEffortChanges({ a: '', b: 'high', c: 'low' }, ['a'], 'b'), {
    a: '',
    b: 'high',
  });
  assert.deepEqual(selectedEffortChanges({ a: 'high' }, ['b'], 'b'), {});
});

test('context overrides distinguish preserve, clear and integer tokens without coercing old values', () => {
  assert.equal(contextWindowText(null), '');
  assert.equal(contextWindowText(undefined), '');
  assert.equal(contextWindowText(200000), '200000');
  assert.equal(contextWindowText('unknown-old-value'), 'unknown-old-value');
  assert.equal(contextWindowText({ future: true }), '{"future":true}');
  assert.equal(contextWindowChange(undefined), undefined);
  assert.deepEqual(contextWindowChange(''), { value: null });
  assert.deepEqual(contextWindowChange('400000'), { value: 400000 });
  assert.equal(parseContextWindow('9007199254740991'), Number.MAX_SAFE_INTEGER);
  for (const invalid of ['0', '-1', '1.5', '200K', '1e6', ' 200000 ', '9007199254740992'])
    assert.throws(() => parseContextWindow(invalid), /上下文窗口/);
  assert.deepEqual(selectedContextChanges({ a: '', b: '200000', removed: 'bad' }, ['a'], 'b'), {
    a: null,
    b: 200000,
  });
  assert.deepEqual(selectedContextChanges({}, ['a'], 'a'), {});
  assert.deepEqual(
    Object.entries(selectedContextChanges({ ['__proto__']: '1' }, ['__proto__'], '')),
    [['__proto__', 1]],
  );
  // A conflicting aliases clear must remain explicit even when its representative is null.
  assert.deepEqual(selectedContextChanges(changeModelEffort({}, 'a', '', '', true), ['a'], 'a'), {
    a: null,
  });
});

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
