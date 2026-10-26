import { reasoningLevelsText } from '../lib/model-settings';
import { ModelSettingField, SettingHelp } from './ModelSettingField';

export function OpenCodeReasoningLevels({
  id,
  model,
  original,
  draft,
  conflict,
  onChange,
  onReset,
  disabled,
  editable,
  inline,
}: {
  id: string;
  model: string;
  original: unknown;
  draft: string | undefined;
  conflict: boolean;
  onChange: (text: string) => void;
  onReset: () => void;
  disabled: boolean;
  editable: boolean;
  inline?: boolean;
}) {
  const changed = draft !== undefined;
  const value = draft ?? reasoningLevelsText(original);
  return (
    <ModelSettingField
      id={id}
      label="可选思考等级"
      summary={conflict && !changed ? '别名声明不同 · 保留原值' : value || '沿用目录'}
      changed={changed}
      disabled={disabled || !editable}
      inline={inline}
    >
      {!editable && (
        <p className="n-config-note">
          档位声明仅支持 OpenCode V2 原生 providers 配置；不会自动迁移旧格式。
        </p>
      )}
      {editable && (
        <>
          <label className={inline ? 'n-sr-only' : undefined} htmlFor={id}>
            {model} 的可选思考等级（英文逗号分隔）
          </label>
          <input
            id={id}
            value={value}
            disabled={disabled}
            spellCheck={false}
            placeholder="例如 low, medium, high；请按服务商实际支持填写"
            onChange={(event) => onChange(event.target.value)}
          />
          <p className="n-config-note">英文逗号分隔，仅声明菜单档位，不设置默认等级。</p>
          <SettingHelp>
            <p className="n-config-note">
              与默认推理等级分开配置。非空时写入 capabilities.reasoning.supported 和 effort，供
              OpenCode 生成可选 variants；不会验证服务商支持情况。留空只移除 effort 声明。
              已有自定义 variants、toggle 和 budget 均保留，因此菜单可能还有额外选项。V2
              自动重载，必要时使用 /reload。
            </p>
          </SettingHelp>
          {conflict && (
            <p className="n-config-note">
              明确修改会更新这个 API 模型 ID 的全部别名，其他模型不变。
            </p>
          )}
          {changed && (
            <button type="button" className="n-inline-action" disabled={disabled} onClick={onReset}>
              撤销档位修改
            </button>
          )}
        </>
      )}
    </ModelSettingField>
  );
}
