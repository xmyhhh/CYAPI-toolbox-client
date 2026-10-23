import type { ServerSkill } from './api/native.ts';

type CatalogEntry = {
  skill: Pick<ServerSkill, 'id' | 'title' | 'description' | 'category'> &
    Partial<Pick<ServerSkill, 'setupNotes' | 'markdown' | 'recommended'>>;
};

export const skillCategory = (skill: { category?: string }) => skill.category?.trim() || '其他';

export function catalogCategories(entries: readonly CatalogEntry[]) {
  const counts = new Map<string, number>();
  for (const { skill } of entries) {
    const category = skillCategory(skill);
    counts.set(category, (counts.get(category) ?? 0) + 1);
  }
  return [...counts].sort(([a], [b]) => a.localeCompare(b, 'zh-CN'));
}

// The searchable text covers the metadata people read first, then the upstream
// SKILL.md body, so trigger words that only exist upstream stay reachable.
function bodyText(skill: CatalogEntry['skill']) {
  return `${skill.title} ${skill.id} ${skill.description} ${skillCategory(skill)} ${(
    skill.setupNotes ?? []
  ).join(' ')} ${skill.markdown ?? ''}`
    .normalize('NFKC')
    .toLocaleLowerCase();
}

// A hit in the name or summary is a stronger signal than a hit buried in the body.
function relevance(skill: CatalogEntry['skill'], terms: string[]) {
  const primary = `${skill.title} ${skill.id} ${skill.description}`
    .normalize('NFKC')
    .toLocaleLowerCase();
  const secondary = `${skillCategory(skill)} ${(skill.setupNotes ?? []).join(' ')}`
    .normalize('NFKC')
    .toLocaleLowerCase();
  let score = 0;
  for (const term of terms) {
    if (primary.includes(term)) score += 3;
    else if (secondary.includes(term)) score += 2;
    else score += 1;
  }
  return score;
}

export function filterCatalog<T extends CatalogEntry>(
  entries: readonly T[],
  query: string,
  category: string,
) {
  const terms = query.normalize('NFKC').trim().toLocaleLowerCase().split(/\s+/).filter(Boolean);
  const matched = entries.filter(({ skill }) => {
    if (category && skillCategory(skill) !== category) return false;
    if (!terms.length) return true;
    const text = bodyText(skill);
    return terms.every((term) => text.includes(term));
  });
  if (!terms.length) return matched;
  return matched
    .map((entry, index) => ({ entry, score: relevance(entry.skill, terms), index }))
    .sort((a, b) => b.score - a.score || a.index - b.index)
    .map(({ entry }) => entry);
}

export function filterRecommended<T extends CatalogEntry>(
  entries: readonly T[],
  recommendedOnly: boolean,
) {
  if (!recommendedOnly) return entries;
  return entries.filter(({ skill }) => skill.recommended === true);
}
