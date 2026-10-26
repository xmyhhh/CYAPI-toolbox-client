import type { CodexConfiguration, CodexModelSetting } from './api/native';
import { configuredModels } from './model-settings.ts';

export class CodexSaveConflict extends Error {}

/** These are recoverable edit conflicts, not failures requiring a log dialog. */
export function codexSaveConflictMessage(action: string, message: string): string | undefined {
  if (action !== 'codex-save') return undefined;
  if (
    message.includes('Codex 的模型、线路或认证设置已被其他程序修改') ||
    message.includes('Codex 配置已被其他程序修改，请重新读取')
  ) {
    return '保存期间 Codex 配置又发生了变化，本次尚未保存，当前输入已保留。请再次保存；如果仍有冲突，请重新读取并核对。';
  }
  return undefined;
}

export interface CodexSaveDraft {
  baseUrl: string;
  apiKey: string;
  model: string;
  models: string[];
  modelSettings: Record<string, CodexModelSetting>;
  reasoningEffort: string;
  contextWindowChange?: { value: number | null };
}

const same = (a: unknown, b: unknown) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);

/** Merge only disjoint edits; fresh revisions never mean permission to overwrite. */
export function prepareCodexSave(
  original: CodexConfiguration,
  latest: CodexConfiguration,
  draft: CodexSaveDraft,
): CodexSaveDraft & { revision: string; saveRevision?: string } {
  if (original.configPath !== latest.configPath || original.authPath !== latest.authPath) {
    throw new CodexSaveConflict(
      'Codex 配置位置已变化，尚未保存。当前输入已保留，请重新读取并核对。',
    );
  }
  const changed =
    original.saveRevision && latest.saveRevision
      ? original.saveRevision !== latest.saveRevision
      : original.revision !== latest.revision;
  if (changed && (!original.rebaseGuard || original.rebaseGuard !== latest.rebaseGuard)) {
    throw new CodexSaveConflict(
      'Codex 的线路或认证配置已在外部更新，尚未保存。当前输入已保留，请重新读取并核对，避免覆盖新设置。',
    );
  }

  const conflicts: string[] = [];
  function merge<T>(label: string, before: T, current: T, edited: T): T {
    if (same(edited, before)) return current;
    if (same(current, before) || same(current, edited)) return edited;
    conflicts.push(label);
    return edited;
  }
  const model = merge('默认模型', original.model, latest.model, draft.model);
  const models = merge(
    '模型列表',
    configuredModels(original.models, original.model),
    configuredModels(latest.models, latest.model),
    configuredModels(draft.models, draft.model),
  );
  const reasoningEffort = merge(
    '全局推理等级',
    original.reasoningEffort,
    latest.reasoningEffort,
    draft.reasoningEffort,
  );
  const modelSettings: Record<string, CodexModelSetting> = {};
  // Null/empty declarations mean "preserve catalog metadata", not a removal.
  // An absent draft must therefore retain newly published model metadata too.
  for (const id of configuredModels(models, model)) {
    const before = original.modelSettings?.[id];
    const current = latest.modelSettings?.[id];
    const edited = draft.modelSettings[id];
    const setting: CodexModelSetting = {};
    for (const key of ['contextWindow', 'reasoningLevels', 'defaultLevel'] as const) {
      const requested = edited?.[key] ?? before?.[key] ?? null;
      Object.assign(setting, {
        [key]: merge(`${id} 的 ${key}`, before?.[key] ?? null, current?.[key] ?? null, requested),
      });
    }
    Object.defineProperty(modelSettings, id, {
      value: setting,
      enumerable: true,
      configurable: true,
    });
  }
  let contextWindowChange = draft.contextWindowChange;
  if (contextWindowChange) {
    const next = merge(
      '全局上下文窗口',
      original.contextWindow ?? null,
      latest.contextWindow ?? null,
      contextWindowChange.value,
    );
    if (same(next, latest.contextWindow)) contextWindowChange = undefined;
  }
  if (conflicts.length) {
    throw new CodexSaveConflict(
      `以下设置同时在本页和外部被修改：${conflicts.join('、')}。尚未保存，当前输入已保留；请重新读取并核对后再保存。`,
    );
  }
  return {
    ...draft,
    model,
    models,
    reasoningEffort,
    modelSettings,
    contextWindowChange,
    revision: latest.revision,
    saveRevision: latest.saveRevision,
  };
}
