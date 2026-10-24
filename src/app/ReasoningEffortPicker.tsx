const efforts = ['', 'none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const labels: Record<string, string> = {
  '': '不覆盖（移除全局显式等级）',
  none: '不思考 · none',
  minimal: '最少 · minimal',
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
      <label htmlFor={id}>全局推理等级（Codex）</label>
      <select id={id} value={value} onChange={(event) => onChange(event.target.value)}>
        {!efforts.includes(value) && <option value={value}>{value}（当前配置）</option>}
        {efforts.map((effort) => (
          <option key={effort} value={effort}>
            {labels[effort]}
          </option>
        ))}
      </select>
      <p className="n-config-note">
        写入 Codex 的全局
        model_reasoning_effort，不是逐模型默认值。候选值不代表每个模型都支持；不会修改模型目录中的能力声明。未知时请不覆盖。
      </p>
    </div>
  );
}
