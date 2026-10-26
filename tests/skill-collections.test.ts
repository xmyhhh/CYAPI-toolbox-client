import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import {
  collectionCatalog,
  collectionCategories,
  collectionForSkill,
  collectionInstallations,
  pathKey,
} from '../src/features/skills/collections.ts';
import type { CatalogSkill, InstalledSkill } from '../src/features/skills/collections.ts';
import {
  installCollection,
  isCollectionCancelled,
  previewCollection,
} from '../src/features/skills/collection-install.ts';
import type { CollectionCall } from '../src/features/skills/collection-install.ts';
import {
  removableCollectionEntries,
  removeCollectionEntries,
} from '../src/features/skills/collection-run.ts';
import type { NativePlan, NativeRecord, ServerSkill, SkillGroup } from '../src/lib/api/native.ts';

const location = { target: 'codex', scope: 'global', projectPath: '' } as const;
function groupsFor(entries: readonly CatalogSkill[]): SkillGroup[] {
  return [
    {
      id: 'research-suite',
      title: 'Research Skills',
      description: 'Server-defined research suite',
      sourceUrl: 'https://example.com/research',
      items: entries.map(({ skill }) => ({ skillId: skill.id })),
    },
  ];
}
function entry(id: string, overrides: Partial<ServerSkill> = {}): CatalogSkill {
  return {
    skill: {
      id,
      title: id,
      description: id,
      version: '1',
      revision: 'revision',
      sourceUrl: `https://github.com/Yuan1z0825/nature-skills/tree/revision/skills/${id}`,
      sha256: `hash-${id}`,
      size: 10,
      fileCount: 1,
      packagePath: '/package',
      requirements: [],
      markdown: `---\nname: ${id}\n---\n`,
      category: '科研与数据分析',
      ...overrides,
    },
    downloaded: false,
    offlineReady: false,
  };
}
function installed(
  skill: CatalogSkill,
  overrides: Partial<NativeRecord> = {},
  managed = true,
): InstalledSkill {
  return {
    record: {
      id: `record-${skill.skill.id}`,
      skillId: skill.skill.id,
      title: skill.skill.title,
      version: '1',
      revision: 'revision',
      target: 'codex',
      scope: 'global',
      path: `C:/Users/test/.codex/skills/${skill.skill.id}`,
      installedAt: 0,
      runtimes: [],
      packageSha256: skill.skill.sha256,
      ...overrides,
    },
    exists: true,
    managed,
  };
}
const plan = (path: string, overrides: Partial<NativePlan> = {}): NativePlan => ({
  path,
  conflict: false,
  offlineReady: false,
  runtimes: [],
  ...overrides,
});

test('the published Nature catalog becomes one suite and retains every module', async (t) => {
  const catalogFile = new URL('../../server/data/catalog.json', import.meta.url);
  // The private server project is not published. Skip when its data files are absent.
  const missing = await stat(catalogFile).then(
    () => false,
    () => true,
  );
  if (missing) return t.skip('跳过：没有本地准备的私有服务端数据');
  const catalog = JSON.parse(await readFile(catalogFile, 'utf8'));
  const entries: CatalogSkill[] = catalog.skills.map((skill: ServerSkill) => ({
    skill,
    downloaded: false,
    offlineReady: false,
  }));
  const members = entries.filter(({ skill }) => collectionForSkill(skill, catalog.skillGroups));
  assert.ok(members.length > 1);
  const items = collectionCatalog(entries, catalog.skillGroups);
  assert.equal(items.length, entries.length - members.length + 1);
  const suite = items.find((item) => item.kind === 'collection');
  assert.ok(suite?.kind === 'collection');
  assert.deepEqual(
    suite.entries.map(({ skill }) => skill.id),
    catalog.skillGroups[0].items.map(({ skillId }: { skillId: string }) => skillId),
  );
  assert.ok(suite.entries.some(({ skill }) => skill.id === 'nature-shared'));
  assert.ok(suite.entries.some(({ skill }) => skill.id === 'nature-proposal-writer'));
  assert.ok(suite.entries.some(({ skill }) => skill.id === 'nature-academic-search'));
  const [search] = collectionCatalog(entries, catalog.skillGroups, 'nature-academic-search');
  assert.ok(search?.kind === 'collection');
  assert.equal(search.entries.length, members.length);
  assert.ok(search.matches.some(({ skill }) => skill.id === 'nature-academic-search'));
});

test('member searches and filters keep the suite and its complete installation membership', () => {
  const entries = [
    entry('nature-reader', { description: '中英文对照', recommended: true }),
    entry('nature-figure', { description: '科研绘图 PyMuPDF', category: '绘图' }),
    entry('nature-shared'),
  ];
  const groups = groupsFor(entries);
  const [match] = collectionCatalog(entries, groups, 'PyMuPDF', '绘图');
  assert.ok(match?.kind === 'collection');
  assert.deepEqual(
    match.matches.map(({ skill }) => skill.id),
    ['nature-figure'],
  );
  assert.equal(match.entries.length, 3);
  const [recommended] = collectionCatalog(entries, groups, '', '', true);
  assert.ok(recommended?.kind === 'collection');
  assert.equal(recommended.matches.length, 1);
  assert.equal(recommended.entries.length, 3);
  assert.equal(collectionCatalog(entries, groups, 'Research Skills').length, 1);
  assert.equal(collectionCatalog(entries, groups, 'missing').length, 0);
  assert.deepEqual(
    new Map(collectionCategories(entries, groups)),
    new Map([
      ['科研与数据分析', 1],
      ['绘图', 1],
    ]),
  );
});

test('server groups choose items explicitly across repositories and leave other items standalone', () => {
  const first = entry('custom-a', { sourceUrl: 'https://github.com/other/repo' });
  const second = entry('custom-b');
  const standalone = entry('nature-reader');
  const entries = [standalone, first, second];
  const groups = groupsFor([second, first]);
  const items = collectionCatalog(entries, groups);
  assert.equal(items[0].kind, 'skill');
  const suite = items[1];
  assert.ok(suite.kind === 'collection');
  assert.equal(suite.collection.title, groups[0].title);
  assert.deepEqual(suite.entries, [second, first]);
  assert.equal(collectionForSkill(standalone.skill, groups), undefined);

  const changed = [{ ...groups[0], title: 'Design Tools', items: [{ skillId: first.skill.id }] }];
  const [renamed] = collectionCatalog(entries, changed, 'Design Tools');
  assert.ok(renamed.kind === 'collection');
  assert.equal(renamed.collection.title, 'Design Tools');
  assert.deepEqual(renamed.entries, [first]);
  assert.equal(collectionCatalog(entries, changed)[2].kind, 'skill');
});

test('without groups even Nature Skills remain standalone and category counts stay distinct', () => {
  const entries = [entry('nature-reader'), entry('research-suite')];
  assert.deepEqual(
    collectionCatalog(entries).map((item) => item.kind),
    ['skill', 'skill'],
  );
  assert.equal(collectionForSkill(entries[0].skill), undefined);
  const installedEntries = entries.map((skill) => installed(skill));
  assert.ok(
    collectionInstallations(installedEntries, entries).every((item) => item.kind === 'skill'),
  );
  assert.equal(
    new Map(collectionCategories(entries, groupsFor([entries[0]]))).get('科研与数据分析'),
    2,
  );
});

test('installed suites separate directories and ownership and recognize upstream aliases', () => {
  const reader = entry('nature-reader');
  const proposal = entry('nature-proposal-writer', { markdown: '---\nname: researchwrite\n---\n' });
  const entries = [reader, proposal];
  const records = [
    installed(reader),
    installed(proposal, { skillId: 'researchwrite' }, false),
    installed(reader, { path: 'F:/project/.agents/skills/nature-reader', scope: 'project' }),
    installed(reader, { path: 'C:/Users/test/.claude/skills/nature-reader', target: 'claude' }),
  ];
  const grouped = collectionInstallations(records, entries, groupsFor(entries));
  assert.equal(grouped.length, 3);
  assert.ok(grouped[0].kind === 'collection');
  assert.equal(grouped[0].entries.length, 2);
  assert.equal(grouped[0].entries[1].managed, false);
  assert.equal(pathKey('C:\\Users\\TEST\\skills\\'), pathKey('c:/users/test/skills'));
  assert.notEqual(pathKey('/Users/Test/skills'), pathKey('/Users/test/skills'));
});

test('preview orders shared dependencies, keeps local versions and updates only owned files', async () => {
  const shared = entry('nature-shared');
  const reader = entry('nature-reader', { sharedDependencies: ['nature-shared'] });
  const local = entry('nature-local');
  const records = [
    installed(reader, { packageSha256: 'old' }),
    installed(local, { packageSha256: 'old' }, false),
  ];
  const call: CollectionCall = async <T>(_action: string, options?: Record<string, unknown>) => {
    const id = options!.skillId;
    return plan(`C:/Users/test/.codex/skills/${id}`, {
      installed: id !== shared.skill.id,
      conflict: id !== shared.skill.id,
    }) as T;
  };
  const steps = await previewCollection([reader, local, shared], records, location, call);
  assert.deepEqual(
    steps.map(({ entry, action }) => [entry.skill.id, action]),
    [
      ['nature-shared', 'install'],
      ['nature-reader', 'update'],
      ['nature-local', 'keep'],
    ],
  );
  const inherited = await previewCollection(
    [reader],
    records.map((record) => ({ ...record, record: { ...record.record, target: 'claude' } })),
    location,
    call,
  );
  assert.equal(inherited[0].action, 'keep');
});

test('conflicts, unavailable runtimes and preview errors block writes', async () => {
  const entries = [entry('conflict'), entry('runtime'), entry('failure')];
  const call: CollectionCall = async <T>(_action: string, options?: Record<string, unknown>) => {
    if (options!.skillId === 'failure') throw new Error('invalid project path');
    return plan(
      'C:/target',
      options!.skillId === 'conflict'
        ? { conflict: true }
        : {
            runtimes: [{ requirement: { kind: 'python', minimum: '3.12' } }],
          },
    ) as T;
  };
  const steps = await previewCollection(entries, [], location, call);
  assert.ok(steps.every((step) => step.action === 'blocked'));
  let wrote = false;
  await assert.rejects(
    installCollection(
      steps,
      location,
      async <T>() => {
        wrote = true;
        return {} as T;
      },
      () => {},
    ),
  );
  assert.equal(wrote, false);
});

test('academic search preserves the exact Python version error for installs and updates', async () => {
  const academic = entry('nature-academic-search');
  const record = installed(academic, { packageSha256: 'old' });
  const error = '离线依赖需要 Python 3.12，未检测到兼容版本';
  const call: CollectionCall = async <T>(_action: string, options?: Record<string, unknown>) => {
    assert.equal(options!.skillId, academic.skill.id);
    return plan(record.record.path, {
      installed: options!.scope === 'global',
      runtimes: [{ requirement: { kind: 'python', minimum: '3.12.0' }, error }],
    }) as T;
  };
  for (const scope of ['global', 'project'] as const) {
    const steps = await previewCollection([academic], [record], { ...location, scope }, call);
    assert.equal(steps[0].action, 'blocked');
    assert.equal(steps[0].note, error);
  }
});

test('a runtime removed after suite preview blocks downloading and installing', async () => {
  const calls: string[] = [];
  const error = '离线依赖需要 Python 3.12，未检测到兼容版本';
  const result = await installCollection(
    [{ entry: entry('nature-academic-search'), action: 'install', note: '' }],
    location,
    async <T>(action: string) => {
      calls.push(action);
      return plan('C:/target', {
        runtimes: [{ requirement: { kind: 'python', minimum: '3.12.0' }, error }],
      }) as T;
    },
    () => {},
  );
  assert.equal(result.installed, 0);
  assert.equal(result.failed!.message, error);
  assert.deepEqual(calls, ['preview']);
});

test('install prepares only missing downloads, preserves target and reports partial failure', async () => {
  const shared = entry('nature-shared');
  const reader = entry('nature-reader', { sharedDependencies: ['nature-shared'] });
  const later = entry('later');
  const calls: { action: string; options?: Record<string, unknown> }[] = [];
  const call: CollectionCall = async <T>(action: string, options?: Record<string, unknown>) => {
    calls.push({ action, options });
    if (action === 'preview')
      return plan(`C:/target/${options!.skillId}`, {
        offlineReady: options!.skillId === shared.skill.id,
      }) as T;
    if (action === 'update-skill') throw new Error('file is in use');
    return {} as T;
  };
  const result = await installCollection(
    [
      { entry: shared, action: 'install', note: '' },
      { entry: reader, action: 'update', note: '' },
      { entry: later, action: 'install', note: '' },
    ],
    { target: 'claude', scope: 'project', projectPath: 'F:/paper' },
    call,
    () => {},
  );
  assert.equal(result.installed, 1);
  assert.equal(result.updated, 0);
  assert.match(result.failed!.message, /file is in use/);
  assert.deepEqual(
    calls.map(({ action, options }) => [action, options!.skillId]),
    [
      ['preview', 'nature-shared'],
      ['install', 'nature-shared'],
      ['preview', 'nature-reader'],
      ['download', 'nature-reader'],
      ['update-skill', 'nature-reader'],
    ],
  );
  assert.equal(calls.find(({ action }) => action === 'install')!.options!.projectPath, 'F:/paper');
  assert.equal(calls.find(({ action }) => action === 'update-skill')!.options!.target, 'claude');
});

test('a retry skips completed installations and a destination race never overwrites files', async () => {
  const reader = entry('nature-reader');
  const calls: string[] = [];
  const call: CollectionCall = async <T>(action: string) => {
    calls.push(action);
    return plan('C:/target', { installed: true, conflict: true }) as T;
  };
  const result = await installCollection(
    [{ entry: reader, action: 'install', note: '' }],
    location,
    call,
    () => {},
  );
  assert.equal(result.kept, 1);
  assert.deepEqual(calls, ['preview']);
  const raced = await installCollection(
    [{ entry: reader, action: 'install', note: '' }],
    location,
    async <T>() => plan('C:/target', { conflict: true }) as T,
    () => {},
  );
  assert.equal(raced.installed, 0);
  assert.match(raced.failed!.message, /目标目录已存在/);
});

test('a cancelled suite stops before the next module and reports no failure', async () => {
  const controller = new AbortController();
  const calls: string[] = [];
  const call: CollectionCall = async <T>(action: string, options?: Record<string, unknown>) => {
    calls.push(`${action}:${options!.skillId}`);
    if (action === 'install') controller.abort();
    return plan(`C:/target/${options!.skillId}`) as T;
  };
  await assert.rejects(
    installCollection(
      [
        { entry: entry('nature-shared'), action: 'install', note: '' },
        { entry: entry('nature-reader'), action: 'install', note: '' },
      ],
      location,
      call,
      () => {},
      controller.signal,
    ),
    (error: unknown) => isCollectionCancelled(error),
  );
  assert.deepEqual(calls, [
    'preview:nature-shared',
    'download:nature-shared',
    'install:nature-shared',
  ]);
});

test('a suite reports the stage of every module while it works', async () => {
  const phases: string[] = [];
  await installCollection(
    [{ entry: entry('nature-reader'), action: 'install', note: '' }],
    location,
    async <T>(action: string) =>
      (action === 'preview' ? plan('C:/target', { offlineReady: false }) : {}) as T,
    (progress) => phases.push(`${progress.index}/${progress.total}:${progress.phase}`),
  );
  assert.deepEqual(phases, ['1/1:check', '1/1:download', '1/1:install']);
});

test('a removal stops at the first failure and keeps the moved modules', async () => {
  const calls: string[] = [];
  const outcome = await removeCollectionEntries({
    entries: [installed(entry('nature-reader')), installed(entry('nature-figure'))],
    workspace: 'codex',
    call: async <T>(_action: string, options?: Record<string, unknown>) => {
      calls.push(options!.recordId);
      if (options!.recordId === 'record-nature-reader') throw new Error('file is in use');
      return {} as T;
    },
    signal: new AbortController().signal,
    onProgress: () => {},
  });
  assert.equal(outcome.removed, 0);
  assert.equal(outcome.failed!.title, 'nature-reader');
  assert.match(outcome.failed!.message, /file is in use/);
  assert.deepEqual(calls, ['record-nature-reader']);
});

test('a cancelled removal leaves the remaining modules in place', async () => {
  const controller = new AbortController();
  const calls: string[] = [];
  await assert.rejects(
    removeCollectionEntries({
      entries: [installed(entry('nature-reader')), installed(entry('nature-figure'))],
      workspace: 'codex',
      call: async <T>(_action: string, options?: Record<string, unknown>) => {
        calls.push(options!.recordId);
        controller.abort();
        return {} as T;
      },
      signal: controller.signal,
      onProgress: () => {},
    }),
    (error: unknown) => isCollectionCancelled(error),
  );
  assert.deepEqual(calls, ['record-nature-reader']);
});

test('all 23 locally installed suite modules are eligible and use confirmed local removal paths', async () => {
  const entries = Array.from({ length: 23 }, (_, index) => ({
    ...installed(entry(`nature-module-${index}`), {}, false),
    removable: true,
  }));
  assert.equal(removableCollectionEntries(entries, 'codex').length, 23);
  const calls: { action: string; options?: Record<string, unknown> }[] = [];
  const progress: number[] = [];
  const outcome = await removeCollectionEntries({
    entries,
    workspace: 'codex',
    call: async <T>(action: string, options?: Record<string, unknown>) => {
      calls.push({ action, options });
      return {} as T;
    },
    signal: new AbortController().signal,
    onProgress: (update) => progress.push(update.total),
  });
  assert.equal(outcome.removed, 23);
  assert.deepEqual(
    outcome.removedIds,
    entries.map(({ record }) => record.id),
  );
  assert.deepEqual(
    calls,
    entries.map(({ record }) => ({
      action: 'remove-local',
      options: { recordId: record.id, installationPath: record.path },
    })),
  );
  assert.ok(progress.every((total) => total === 23));
});

test('mixed suite removal includes local modules but never writes protected, missing or inherited entries', async () => {
  const managed = installed(entry('managed'));
  const local = { ...installed(entry('local'), {}, false), removable: true };
  const retained = [
    {
      ...local,
      record: { ...local.record, id: 'system' },
      removalBlockedReason: '系统内置 Skill 受保护',
    },
    { ...local, record: { ...local.record, id: 'missing' }, exists: false },
    { ...local, record: { ...local.record, id: 'inherited', target: 'claude' as const } },
    { ...local, record: { ...local.record, id: 'blocked' }, removable: false },
    { ...local, record: { ...local.record, id: 'old-backend' }, removable: undefined },
    { ...managed, record: { ...managed.record, id: 'protected-managed' }, removable: false },
  ];
  const entries = [managed, local, ...retained];
  assert.deepEqual(removableCollectionEntries(entries, 'codex'), [managed, local]);
  const calls: { action: string; options?: Record<string, unknown> }[] = [];
  const outcome = await removeCollectionEntries({
    entries,
    workspace: 'codex',
    call: async <T>(action: string, options?: Record<string, unknown>) => {
      calls.push({ action, options });
      return {} as T;
    },
    signal: new AbortController().signal,
    onProgress: () => {},
  });
  assert.equal(outcome.removed, 2);
  assert.deepEqual(calls, [
    { action: 'remove', options: { recordId: managed.record.id } },
    {
      action: 'remove-local',
      options: { recordId: local.record.id, installationPath: local.record.path },
    },
  ]);
});

test('a local removal failure preserves completed modules and stops before the next module', async () => {
  const entries = ['first', 'locked', 'later'].map((id) => ({
    ...installed(entry(id), {}, false),
    removable: true,
  }));
  const calls: string[] = [];
  const outcome = await removeCollectionEntries({
    entries,
    workspace: 'codex',
    call: async <T>(action: string, options?: Record<string, unknown>) => {
      assert.equal(action, 'remove-local');
      calls.push(options!.recordId);
      if (options!.recordId === entries[1].record.id) throw new Error('file is in use');
      return {} as T;
    },
    signal: new AbortController().signal,
    onProgress: () => {},
  });
  assert.equal(outcome.removed, 1);
  assert.deepEqual(outcome.removedIds, [entries[0].record.id]);
  assert.equal(outcome.failed!.title, 'locked');
  assert.deepEqual(
    calls,
    entries.slice(0, 2).map(({ record }) => record.id),
  );
});

test('cancelling a local batch leaves later modules untouched', async () => {
  const controller = new AbortController();
  const calls: string[] = [];
  const entries = ['first', 'later'].map((id) => ({
    ...installed(entry(id), {}, false),
    removable: true,
  }));
  await assert.rejects(
    removeCollectionEntries({
      entries,
      workspace: 'codex',
      call: async <T>(action: string, options?: Record<string, unknown>) => {
        assert.equal(action, 'remove-local');
        calls.push(options!.recordId);
        controller.abort();
        return {} as T;
      },
      signal: controller.signal,
      onProgress: () => {},
    }),
    (error: unknown) => isCollectionCancelled(error),
  );
  assert.deepEqual(calls, [entries[0].record.id]);
});
