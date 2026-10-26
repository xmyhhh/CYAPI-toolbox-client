/** Model IDs are API identifiers, not display names or OpenCode aliases. */
export function configuredModels(models: string[], defaultModel: string): string[] {
  return [...new Set([...models, defaultModel].map((id) => id.trim()).filter(Boolean))];
}

export function modelEffort(
  model: string,
  efforts: Record<string, string>,
  keys: Record<string, string> = {},
): string {
  const key = Object.hasOwn(keys, model) ? keys[model] : model;
  return Object.hasOwn(efforts, key) ? efforts[key] : '';
}

/** Resolve newly added full Claude IDs against existing keys without guessing alias releases. */
export function claudeModelEffort(
  model: string,
  efforts: Record<string, string>,
  keys: Record<string, string> = {},
): string {
  if (Object.hasOwn(keys, model) || Object.hasOwn(efforts, model)) {
    return modelEffort(model, efforts, keys);
  }
  if (!model.startsWith('claude-')) return '';
  const withoutContext = (id: string) => id.replace(/\[1m\]$/, '');
  const identity = (id: string) => withoutContext(id).replace(/-\d{8}$/, '');
  for (const candidate of [withoutContext(model), identity(model)]) {
    if (Object.hasOwn(efforts, candidate)) return efforts[candidate];
  }
  const matches = Object.keys(efforts).filter((key) => identity(key) === identity(model));
  return matches.length === 1 ? efforts[matches[0]] : '';
}

/** Missing preserves; empty removes. Force an explicit patch when aliases have no common original. */
export function changeModelEffort(
  changes: Record<string, string>,
  model: string,
  value: string,
  original: string,
  force = false,
): Record<string, string> {
  const next = { ...changes, [model]: value };
  if (!force && value === original) delete next[model];
  return next;
}

/** Keep original JSON values opaque until the user explicitly edits this setting. */
export function contextWindowText(value: unknown): string {
  if (value == null) return '';
  return typeof value === 'string' ? value : JSON.stringify(value);
}

export function parseContextWindow(value: string): number | null {
  if (value === '') return null;
  if (!/^\d+$/.test(value)) throw new Error('上下文窗口必须是正整数（tokens），或留空移除覆盖。');
  const number = Number(value);
  if (!Number.isSafeInteger(number) || number <= 0)
    throw new Error('上下文窗口须为 1 到 9007199254740991 之间的整数（tokens）。');
  return number;
}

/** Undefined preserves; an explicit empty draft clears only this override. */
export function contextWindowChange(
  value: string | undefined,
): { value: number | null } | undefined {
  return value === undefined ? undefined : { value: parseContextWindow(value) };
}

export function selectedContextChanges(
  changes: Record<string, string>,
  models: string[],
  defaultModel: string,
): Record<string, number | null> {
  const selected = new Set(configuredModels(models, defaultModel));
  return Object.fromEntries(
    Object.entries(changes)
      .filter(([id]) => selected.has(id))
      .map(([id, value]) => [id, parseContextWindow(value)]),
  );
}

export function selectedEffortChanges(
  changes: Record<string, string>,
  models: string[],
  defaultModel: string,
): Record<string, string> {
  const selected = new Set(configuredModels(models, defaultModel));
  return Object.fromEntries(Object.entries(changes).filter(([id]) => selected.has(id)));
}

export function reasoningLevelsText(value: unknown): string {
  if (value == null) return '';
  return Array.isArray(value) && value.every((level) => typeof level === 'string')
    ? value.join(', ')
    : JSON.stringify(value);
}

/** Explicit capability edits only; a blank field removes the effort declaration. */
export function selectedReasoningLevelChanges(
  changes: Record<string, string>,
  models: string[],
  defaultModel: string,
): Record<string, string[] | null> {
  const selected = new Set(configuredModels(models, defaultModel));
  return Object.fromEntries(
    Object.entries(changes)
      .filter(([id]) => selected.has(id))
      .map(([id, text]) => {
        if (!text.trim()) return [id, null];
        const levels = text.split(',').map((level) => level.trim());
        if (
          levels.length > 64 ||
          new Set(levels).size !== levels.length ||
          levels.some(
            (level) =>
              !level ||
              new TextEncoder().encode(level).length > 64 ||
              /[\s\u0000-\u001f\u007f-\u009f]/u.test(level),
          )
        )
          throw new Error(
            `${id} 的可选思考等级须以英文逗号分隔，最多 64 项，每项不超过 64 字节，不能重复、为空或包含空白和控制字符。`,
          );
        return [id, levels];
      }),
  );
}
