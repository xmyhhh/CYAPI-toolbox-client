import { useState } from 'react';
import type { CodexModelSetting } from '../lib/api/native';
import { parseContextWindow } from '../lib/model-settings';

const levelPresets = [
  'none',
  'minimal',
  'low',
  'medium',
  'high',
  'xhigh',
  'max',
  'ultra',
  'persistent',
];

export interface CodexSettingDraft {
  contextWindow: string;
  levels: string[];
  defaultLevel: string;
}

export const emptySettingDraft: CodexSettingDraft = {
  contextWindow: '',
  levels: [],
  defaultLevel: '',
};

/** Drafts mirror the published catalog per model; absent entries stay undeclared. */
export function settingDrafts(
  view: Record<string, CodexModelSetting> | undefined,
): Record<string, CodexSettingDraft> {
  return Object.fromEntries(
    Object.entries(view ?? {}).map(([model, setting]) => [
      model,
      {
        contextWindow: setting.contextWindow == null ? '' : String(setting.contextWindow),
        levels: setting.reasoningLevels ?? [],
        defaultLevel: setting.defaultLevel ?? '',
      },
    ]),
  );
}

/**
 * Only configured models are sent back. An empty context window or level list
 * serializes as null, meaning "keep the published catalog's own metadata".
 */
export function settingsPayload(
  drafts: Record<string, CodexSettingDraft>,
  models: string[],
): Record<string, CodexModelSetting> {
  const selected = new Set(models);
  return Object.fromEntries(
    Object.entries(drafts)
      .filter(([model]) => selected.has(model))
      .map(([model, draft]) => {
        const trimmed = draft.contextWindow.trim();
        return [
          model,
          {
            contextWindow: trimmed === '' ? null : parseContextWindow(trimmed),
            reasoningLevels: draft.levels.length ? draft.levels : null,
            defaultLevel: draft.defaultLevel.trim() || null,
          },
        ] as [string, CodexModelSetting];
      }),
  );
}

function orderLevels(levels: string[]): string[] {
  const known = levelPresets.filter((level) => levels.includes(level));
  return [...known, ...levels.filter((level) => !levelPresets.includes(level)).sort()];
}

function sameDraft(original: CodexSettingDraft | undefined, draft: CodexSettingDraft): boolean {
  if (!original) return false;
  const a = orderLevels(original.levels);
  const b = orderLevels(draft.levels);
  return (
    original.contextWindow === draft.contextWindow &&
    original.defaultLevel === draft.defaultLevel &&
    a.length === b.length &&
    a.every((level, index) => level === b[index])
  );
}

/** Declares one model's context window and reasoning levels into the catalog. */
export function CodexModelSettings({
  id,
  model,
  draft,
  original,
  onChange,
  onReset,
  disabled,
}: {
  id: string;
  model: string;
  draft: CodexSettingDraft;
  original: CodexSettingDraft | undefined;
  onChange: (draft: CodexSettingDraft) => void;
  onReset: () => void;
  disabled?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const [custom, setCustom] = useState('');
  const [error, setError] = useState('');
  const changed = !sameDraft(original, draft);
  const levels = orderLevels(draft.levels);
  const options = [
    ...levelPresets,
    ...levels.filter((level) => !levelPresets.includes(level)),
  ];
  const summary = [
    draft.contextWindow.trim() ? `窗口 ${draft.contextWindow.trim()}` : '窗口不声明',
    levels.length ? `等级 ${levels.join(' / ')}` : '等级不声明',
    draft.defaultLevel ? `默认 ${draft.defaultLevel}` : '默认自动',
  ].join(' · ');

  function toggle(level: string, checked: boolean) {
    const next = checked
      ? [...draft.levels, level]
      : draft.levels.filter((existing) => existing !== level);
    onChange({
      ...draft,
      levels: next,
      defaultLevel:
        draft.defaultLevel && !next.includes(draft.defaultLevel) ? '' : draft.defaultLevel,
    });
    setError('');
  }

  function addCustom() {
    const level = custom.trim();
    if (!level || draft.levels.includes(level)) return;
    if (level.length > 64 || /\s/.test(level) || /[\u0000-\u001f\u007f-\u009f]/u.test(level)) {
      setError('思考等级不能为空、包含空白或控制字符，且不超过 64 个字符。');
      return;
    }
    onChange({ ...draft, levels: [...draft.levels, level] });
    setCustom('');
    setError('');
  }

  return (
    <div className="n-model-reasoning">
      <div className="n-model-effort-summary">
        <span>
          逐模型目录声明：<strong>{summary}</strong>
        </span>
        {changed && <span className="n-model-default">待保存</span>}
        <button
          type="button"
          className="n-inline-action"
          disabled={disabled}
          aria-expanded={editing}
          aria-controls={`${id}-editor`}
          onClick={() => setEditing(!editing)}
          aria-label={`配置 ${model} 的上下文窗口与思考等级`}
        >
          {editing ? '收起' : '配置'}
        </button>
      </div>
      {editing && (
        <div className="n-model-effort-editor n-config-field" id={`${id}-editor`}>
          <div className="n-config-field">
            <label htmlFor={`${id}-context`}>{model} 的上下文窗口（tokens）</label>
            <input
              id={`${id}-context`}
              inputMode="numeric"
              value={draft.contextWindow}
              placeholder="留空不声明，沿用目录或工具原值"
              spellCheck={false}
              disabled={disabled}
              onChange={(event) => onChange({ ...draft, contextWindow: event.target.value })}
            />
          </div>
          <fieldset disabled={disabled} className="n-codex-levels">
            <legend>思考等级声明（写入模型目录，Codex 据此显示可挡选等级）</legend>
            {options.map((level) => (
              <label key={level}>
                <input
                  type="checkbox"
                  checked={levels.includes(level)}
                  disabled={disabled}
                  onChange={(event) => toggle(level, event.target.checked)}
                />
                <span>{level}</span>
              </label>
            ))}
          </fieldset>
          <div className="n-model-manual">
            <input
              aria-label={`给 ${model} 添加自定义思考等级`}
              value={custom}
              spellCheck={false}
              maxLength={64}
              disabled={disabled}
              onChange={(event) => setCustom(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  addCustom();
                }
              }}
              placeholder="添加自定义思考等级"
            />
            <button
              type="button"
              className="n-button"
              disabled={disabled || !custom.trim()}
              onClick={addCustom}
            >
              添加
            </button>
          </div>
          <div className="n-config-field">
            <label htmlFor={`${id}-default`}>默认思考等级</label>
            <select
              id={`${id}-default`}
              value={draft.defaultLevel}
              disabled={disabled}
              onChange={(event) => {
                const level = event.target.value;
                onChange({
                  ...draft,
                  defaultLevel: level,
                  levels:
                    level && !draft.levels.includes(level)
                      ? [...draft.levels, level]
                      : draft.levels,
                });
              }}
            >
              <option value="">自动（不声明默认等级）</option>
              {levels.map((level) => (
                <option key={level} value={level}>
                  {level}
                </option>
              ))}
            </select>
          </div>
          <p className="n-config-note">
            声明会写入 Codex 模型目录，供该模型挡选。候选只代表可声明的值，不代表服务商真的支持；不确定请留空，继续沿用目录原始元数据。保存后的全局推理等级会覆盖这里的默认值。
          </p>
          {changed && (
            <button
              type="button"
              className="n-inline-action"
              disabled={disabled}
              onClick={onReset}
            >
              撤销此项修改
            </button>
          )}
        </div>
      )}
      {error && (
        <p className="n-config-note n-error-text" role="alert">
          {error}
        </p>
      )}
    </div>
  );
}
