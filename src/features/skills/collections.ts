import { filterCatalog, filterRecommended, skillCategory } from '../../lib/catalog.ts';
import type { NativeRecord, NativeStatus, ServerSkill, SkillGroup } from '../../lib/api/native.ts';

export type CatalogSkill = NativeStatus['skills'][number];
export type InstalledSkill = NativeStatus['installed'][number];
export type SkillCollection = SkillGroup;

export function collectionForSkill(
  skill: Pick<ServerSkill, 'id'>,
  groups: readonly SkillGroup[] = [],
) {
  return groups.find((group) => group.items.some(({ skillId }) => skillId === skill.id));
}

export function collectionEntries(entries: readonly CatalogSkill[], group?: SkillGroup) {
  const byId = new Map(entries.map((entry) => [entry.skill.id, entry]));
  return (group?.items ?? []).flatMap(({ skillId }) => {
    const entry = byId.get(skillId);
    return entry ? [entry] : [];
  });
}

export type CatalogItem =
  | { kind: 'skill'; entry: CatalogSkill }
  | {
      kind: 'collection';
      collection: SkillCollection;
      entries: CatalogSkill[];
      matches: CatalogSkill[];
    };

export function collectionCatalog(
  entries: readonly CatalogSkill[],
  collections: readonly SkillGroup[] = [],
  query = '',
  category = '',
  recommendedOnly = false,
): CatalogItem[] {
  // Include the suite's name in searches while retaining each member's own
  // category, recommendation and upstream trigger words.
  const searchable = entries.map((entry) => {
    const collection = collectionForSkill(entry.skill, collections);
    return collection
      ? {
          ...entry,
          skill: {
            ...entry.skill,
            description: `${entry.skill.description} ${collection.title} ${collection.id}`,
          },
        }
      : entry;
  });
  const byId = new Map(entries.map((entry) => [entry.skill.id, entry]));
  const groups = new Map<string, Extract<CatalogItem, { kind: 'collection' }>>();
  const result: CatalogItem[] = [];
  for (const match of filterCatalog(
    filterRecommended(searchable, recommendedOnly),
    query,
    category,
  )) {
    const entry = byId.get(match.skill.id)!;
    const collection = collectionForSkill(entry.skill, collections);
    if (!collection) {
      result.push({ kind: 'skill', entry });
      continue;
    }
    let group = groups.get(collection.id);
    if (!group) {
      group = {
        kind: 'collection',
        collection,
        entries: collectionEntries(entries, collection),
        matches: [],
      };
      groups.set(collection.id, group);
      result.push(group);
    }
    group.matches.push(entry);
  }
  return result;
}

export function collectionCategories(
  entries: readonly CatalogSkill[],
  collections: readonly SkillGroup[] = [],
) {
  const categories = new Map<string, Set<string>>();
  for (const { skill } of entries) {
    const category = skillCategory(skill);
    const ids = categories.get(category) ?? new Set<string>();
    const collection = collectionForSkill(skill, collections);
    ids.add(collection ? `group:${collection.id}` : `skill:${skill.id}`);
    categories.set(category, ids);
  }
  return [...categories]
    .map(([name, ids]): [string, number] => [name, ids.size])
    .sort(([a], [b]) => a.localeCompare(b, 'zh-CN'));
}

export function pathKey(path: string) {
  const normalized = path.replaceAll('\\', '/').replace(/\/+$/, '');
  return /^[a-z]:\//i.test(normalized) || normalized.startsWith('//')
    ? normalized.toLowerCase()
    : normalized;
}

export function skillForRecord(record: NativeRecord, entries: readonly CatalogSkill[]) {
  return (
    entries.find(({ skill }) => skill.id === record.skillId) ??
    entries.find(({ skill }) => {
      // Local discovery uses the upstream frontmatter name, e.g. researchwrite,
      // which can differ from the catalog's nature-proposal-writer identifier.
      const frontmatter = skill.markdown.match(/^---\r?\n([\s\S]*?)\r?\n---/);
      return frontmatter?.[1].match(/^name:\s*["']?([\w-]+)["']?\s*$/m)?.[1] === record.skillId;
    })
  );
}

/**
 * The modules of one suite that live in a given target and scope. A global
 * installation and a project's installation are separate suites, so the path
 * of the first module decides which one is meant.
 */
export function collectionInstalledEntries(
  next: NativeStatus,
  collection: SkillCollection,
  target: NativeRecord['target'],
  scope: NativeRecord['scope'],
  skillPath?: string,
): InstalledSkill[] {
  const root = skillPath?.replaceAll('\\', '/').replace(/\/[^/]+\/?$/, '');
  const rootKey = root ? pathKey(root) : undefined;
  const match = collectionInstallations(next.installed, next.skills, next.skillGroups).find(
    (item) =>
      item.kind === 'collection' &&
      item.collection.id === collection.id &&
      item.entries[0]?.record.target === target &&
      item.entries[0]?.record.scope === scope &&
      (!rootKey || pathKey(item.root) === rootKey),
  );
  return match?.kind === 'collection' ? match.entries : [];
}

export type InstalledItem =
  | { kind: 'skill'; entry: InstalledSkill }
  | {
      kind: 'collection';
      key: string;
      collection: SkillCollection;
      entries: InstalledSkill[];
      root: string;
    };

export function collectionInstallations(
  installed: readonly InstalledSkill[],
  catalog: readonly CatalogSkill[],
  collections: readonly SkillGroup[] = [],
): InstalledItem[] {
  const groups = new Map<string, Extract<InstalledItem, { kind: 'collection' }>>();
  const result: InstalledItem[] = [];
  for (const entry of installed) {
    const skill = skillForRecord(entry.record, catalog)?.skill;
    const collection = skill && collectionForSkill(skill, collections);
    if (!collection) {
      result.push({ kind: 'skill', entry });
      continue;
    }
    const root = entry.record.path.replaceAll('\\', '/').replace(/\/[^/]+\/?$/, '');
    // A global installation and a different project's installation are separate
    // suites; inherited directories retain their original management target.
    const key = `${collection.id}:${entry.record.target}:${entry.record.scope}:${pathKey(root)}`;
    let group = groups.get(key);
    if (!group) {
      group = { kind: 'collection', key, collection, entries: [], root };
      groups.set(key, group);
      result.push(group);
    }
    group.entries.push(entry);
  }
  return result;
}
