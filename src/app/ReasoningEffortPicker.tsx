const openaiEfforts = ['', 'none', 'minimal', 'low', 'medium', 'high', 'xhigh', 'max', 'ultra'];
const claudeEfforts = ['', 'low', 'medium', 'high', 'xhigh'];
const labels: Record<string, string> = {
  '': '默认（由模型决定）',
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
  api = 'openai',
}: {
  id: string;
  value: string;
  onChange: (value: string) => void;
  api?: 'openai' | 'anthropic';
}) {
  const efforts = api === 'anthropic' ? claudeEfforts : openaiEfforts;
  return (
    <div className="n-config-field">
      <label htmlFor={id}>思考等级</label>
      <select id={id} value={value} onChange={(event) => onChange(event.target.value)}>
        {!efforts.includes(value) && <option value={value}>{value}（当前配置）</option>}
        {efforts.map((effort) => (
          <option key={effort} value={effort}>
            {labels[effort]}
          </option>
        ))}
      </select>
      <p className="n-config-note">对所选模型生效；支持的等级取决于模型和服务商。</p>
    </div>
  );
}
