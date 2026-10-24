import { useState } from 'react';

const presets = ['none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const claudePresets = ['low', 'medium', 'high', 'xhigh'];

/** Options here are editable request values, never an inferred capability declaration. */
export function ModelReasoningSettings({
  id,
  model,
  value,
  original,
  onChange,
  onReset = () => onChange(original),
  api,
  conflict = false,
  changed = value !== original,
  disabled,
}: {
  id: string;
  model: string;
  value: string;
  original: string;
  onChange: (value: string) => void;
  onReset?: () => void;
  api: 'openai' | 'anthropic';
  conflict?: boolean;
  changed?: boolean;
  disabled?: boolean;
}) {
  const [editing, setEditing] = useState(false);
  const choices = api === 'anthropic' ? claudePresets : presets;
  return (
    <div className="n-model-reasoning">
      <div className="n-model-effort-summary">
        <span>
          默认推理等级：
          <strong>
            {conflict && !changed
              ? '多个别名设置不同（保留原值）'
              : value || '不覆盖（继承上层或工具默认）'}
          </strong>
        </span>
        {changed && <span className="n-model-default">待保存</span>}
        <button
          type="button"
          className="n-inline-action"
          disabled={disabled}
          aria-expanded={editing}
          aria-controls={`${id}-editor`}
          onClick={() => setEditing(!editing)}
          aria-label={`配置 ${model} 的默认推理等级`}
        >
          {editing ? '收起' : '配置'}
        </button>
      </div>
      {editing && (
        <div className="n-model-effort-editor" id={`${id}-editor`}>
          <label htmlFor={id}>仅设置 {model} 的默认推理等级</label>
          {api === 'anthropic' ? (
            <select
              id={id}
              value={value}
              disabled={disabled}
              onChange={(event) => onChange(event.target.value)}
            >
              <option value="">不覆盖（移除此模型的显式等级）</option>
              {value && !choices.includes(value) && (
                <option value={value}>{value}（原配置，未验证）</option>
              )}
              {choices.map((effort) => (
                <option value={effort} key={effort}>
                  {effort}
                </option>
              ))}
            </select>
          ) : (
            <>
              <input
                id={id}
                list={`${id}-suggestions`}
                value={value}
                maxLength={64}
                pattern={changed ? '\\S+' : undefined}
                placeholder="留空移除此模型的显式等级"
                spellCheck={false}
                disabled={disabled}
                onChange={(event) => onChange(event.target.value)}
              />
              <datalist id={`${id}-suggestions`}>
                {choices.map((effort) => (
                  <option value={effort} key={effort} />
                ))}
              </datalist>
            </>
          )}
          <p className="n-config-note">
            {api === 'anthropic'
              ? '需要支持 modelSettings 的 Claude Code（v2.1.251+）。这些是可保存的配置值，不代表当前模型或网关都支持；未确认时请不覆盖。别名需要先映射到完整模型 ID。'
              : '手动指定，高级设置：候选值仅供输入参考，当前模型及网关支持情况未知。不覆盖不会关闭思考；会话中选择的 variant 可覆盖此值。'}
          </p>
          {conflict && (
            <p className="n-config-note">
              同一 API 模型 ID 对应多个别名，原等级不同。未编辑时分别保留；明确设置或移除会更新该 ID
              的全部别名，其他模型及 variants 不变。
            </p>
          )}
          {(value || (conflict && !changed)) && (
            <button
              type="button"
              className="n-inline-action"
              disabled={disabled}
              onClick={() => onChange('')}
            >
              移除此模型的等级覆盖
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
