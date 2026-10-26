const efforts = ['', 'none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const labels: Record<string, string> = {
  '': '不覆盖（移除全局显式等级）',
  none: '不思考 · none（多数模型不支持）',
  minimal: '最少 · minimal（部分模型不支持）',
  low: '低 · low',
  medium: '中 · medium',
  high: '高 · high',
  xhigh: '更高 · xhigh',
  max: '最高 · max',
  ultra: '极高 · ultra',
};

export function ReasoningEffortPicker({
  id,
  value,
  onChange,
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
}) {
  return (
    <div className="n-config-field">
      <label htmlFor={id}>推理等级</label>
      <select id={id} value={value} onChange={(event) => onChange(event.target.value)}>
        {!efforts.includes(value) && <option value={value}>{value}（当前配置）</option>}
        {efforts.map((effort) => (
          <option key={effort} value={effort}>
            {labels[effort]}
          </option>
        ))}
      </select>
      <p className="n-config-note">覆盖所有模型的默认等级，需服务商支持。</p>
      <SettingHelp>
        <p className="n-config-note">
          写入 Codex 的全局
          model_reasoning_effort，会覆盖模型目录中逐模型声明的默认等级（可在每个模型的“配置”里声明）。候选值不代表每个模型都支持——例如服务商会直接拒绝
          none 并报“Unsupported value”错误。不确定时请选“不覆盖”。
        </p>
      </SettingHelp>
    </div>
  );
}
import { SettingHelp } from './ModelSettingField';
