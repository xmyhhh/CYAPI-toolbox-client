import { useState } from 'react';
import { contextWindowText } from '../lib/model-settings';

/** Explicit override only: showing or expanding an existing value never creates a patch. */
export function ContextWindowSettings({
  id,
  scope,
  original,
  draft,
  onChange,
  onReset,
  conflict = false,
  disabled,
  children,
}: {
  id: string;
  scope: string;
  original: unknown;
  draft: string | undefined;
  onChange: (value: string) => void;
  onReset: () => void;
  conflict?: boolean;
  disabled?: boolean;
  children?: React.ReactNode;
}) {
  const [editing, setEditing] = useState(false);
  const changed = draft !== undefined;
  const value = draft ?? contextWindowText(original);
  return (
    <div className="n-model-reasoning n-context-window">
      <div className="n-model-effort-summary">
        <span>
          {scope}上下文窗口：
          <strong>
            {conflict && !changed
              ? '多个别名设置不同（保留原值）'
              : value
                ? `${value} tokens${changed ? '' : '（原配置）'}`
                : '不覆盖（使用工具或模型原值）'}
          </strong>
        </span>
        {changed && <span className="n-model-default">待保存</span>}
        <button
          type="button"
          className="n-inline-action"
          disabled={disabled}
          aria-expanded={editing}
          aria-controls={`${id}-editor`}
          aria-label={`配置${scope}上下文窗口`}
          onClick={() => setEditing(!editing)}
        >
          {editing ? '收起' : '配置'}
        </button>
      </div>
      {editing && (
        <div className="n-model-effort-editor n-config-field" id={`${id}-editor`}>
          <label htmlFor={id}>{scope}上下文窗口（tokens）</label>
          <input
            id={id}
            type="text"
            inputMode="numeric"
            value={value}
            pattern={changed ? '[0-9]+' : undefined}
            placeholder="例如 200000；留空移除覆盖"
            spellCheck={false}
            disabled={disabled}
            aria-describedby={`${id}-help`}
            onChange={(event) => onChange(event.target.value)}
          />
          <p className="n-config-note" id={`${id}-help`}>
            请填写服务商确认的实际上下文窗口，单位为 tokens，不是字符或
            K。填大数值不能扩展模型能力；不是最大输出长度或自动压缩阈值。未编辑保留原值。
          </p>
          {children}
          {conflict && (
            <p className="n-config-note">
              同一 API ID 的多个别名上下文设置不同，未编辑时分别保留；明确设置或移除会更新该 ID
              的全部别名，不影响其他模型和最大输出长度。
            </p>
          )}
          {(value || (conflict && !changed)) && (
            <button
              type="button"
              className="n-inline-action"
              disabled={disabled}
              onClick={() => onChange('')}
            >
              移除此项上下文覆盖
            </button>
          )}
          {changed && (
            <button type="button" className="n-inline-action" disabled={disabled} onClick={onReset}>
              撤销此项修改
            </button>
          )}
        </div>
      )}
    </div>
  );
}
