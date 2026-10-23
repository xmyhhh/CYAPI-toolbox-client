import { useEffect, useRef, useState } from 'react';
import { LoaderCircle, Plus, RefreshCw } from 'lucide-react';
import { nativeAvailable, nativeCall } from '../lib/api/native';

type ModelApi = 'openai' | 'anthropic';

interface ModelPickerProps {
  id: string;
  value: string;
  onChange: (value: string) => void;
  selectedModels: string[];
  onModelsChange: (models: string[]) => void;
  baseUrl: string;
  apiKey: string;
  api: ModelApi;
  disabled?: boolean;
  required?: boolean;
  placeholder?: string;
}

interface ModelList {
  models: string[];
  endpoint: string;
}

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

export function ModelPicker({
  id,
  value,
  onChange,
  selectedModels,
  onModelsChange,
  baseUrl,
  apiKey,
  api,
  disabled,
  required,
  placeholder,
}: ModelPickerProps) {
  const [models, setModels] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [fetched, setFetched] = useState(false);
  const [manual, setManual] = useState('');
  const [search, setSearch] = useState('');
  const request = useRef(0);

  useEffect(() => {
    request.current++;
    setLoading(false);
    setModels([]);
    setError('');
    setFetched(false);
    setSearch('');
    return () => {
      request.current++;
    };
  }, [baseUrl, apiKey, api]);

  const canFetch = nativeAvailable && !disabled && !!baseUrl.trim() && !!apiKey.trim();
  const defaultModel = value.trim();
  const selected = [
    ...new Set([...selectedModels, defaultModel].map((model) => model.trim()).filter(Boolean)),
  ];
  const available = [...new Set([...selected, ...models])];
  const visible = available.filter((model) =>
    model.toLowerCase().includes(search.trim().toLowerCase()),
  );

  function chooseDefault(model: string) {
    onModelsChange([...new Set([...selected, model])]);
    onChange(model);
  }

  function toggle(model: string, checked: boolean) {
    if (checked) {
      onModelsChange([...new Set([...selected, model])]);
      if (!defaultModel) onChange(model);
    } else {
      onModelsChange(selected.filter((candidate) => candidate !== model));
    }
  }

  function addManual() {
    const model = manual.trim();
    if (!model) return;
    toggle(model, true);
    setManual('');
    setSearch('');
  }

  async function fetchModels() {
    if (!canFetch) return;
    const current = ++request.current;
    setLoading(true);
    setError('');
    try {
      const result = await nativeCall<ModelList>('models-list', {
        baseUrl,
        apiKey,
        modelApi: api,
      });
      if (request.current !== current) return;
      setModels(result.models);
      setFetched(true);
      if (!result.models.length) setError('服务没有返回可用模型。');
    } catch (error) {
      if (request.current !== current) return;
      setModels([]);
      setFetched(false);
      setError(errorText(error));
    } finally {
      if (request.current === current) setLoading(false);
    }
  }

  return (
    <div className="n-config-field n-model-field">
      <div className="n-config-label-row">
        <label htmlFor={id}>默认模型</label>
        <button
          type="button"
          className="n-inline-action"
          disabled={!canFetch || loading}
          onClick={() => void fetchModels()}
          title="通过 API 的 /models 接口获取模型列表"
        >
          {loading ? <LoaderCircle size={12} className="n-spin" /> : <RefreshCw size={12} />}
          {loading ? '正在获取' : '获取模型列表'}
        </button>
      </div>
      <input
        id={id}
        required={required}
        value={value}
        onChange={(event) => onChange(event.target.value)}
        placeholder={placeholder}
        spellCheck={false}
        disabled={disabled}
        maxLength={200}
      />
      <div className="n-model-manual">
        <input
          aria-label="手动添加可用模型"
          value={manual}
          onChange={(event) => setManual(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter') {
              event.preventDefault();
              addManual();
            }
          }}
          placeholder="输入其他模型 ID"
          spellCheck={false}
          maxLength={200}
          disabled={disabled}
        />
        <button
          type="button"
          className="n-button"
          onClick={addManual}
          disabled={disabled || !manual.trim()}
        >
          <Plus size={13} />
          添加
        </button>
      </div>
      {available.length > 0 && (
        <div className="n-model-choices">
          <p className="n-config-note">
            可用模型 · 已选 {selected.length} 个。勾选可添加，点「设为默认」切换默认模型。
          </p>
          {(available.length > 8 || search) && (
            <input
              aria-label="筛选模型"
              placeholder="搜索模型 ID"
              value={search}
              onChange={(event) => setSearch(event.target.value)}
              disabled={disabled}
            />
          )}
          <div className="n-model-list" role="group" aria-label="可用模型（可多选）">
            {visible.map((model) => (
              <div key={model} className="n-model-choice">
                <label>
                  <input
                    type="checkbox"
                    checked={selected.includes(model)}
                    disabled={disabled || model === defaultModel}
                    onChange={(event) => toggle(model, event.target.checked)}
                  />
                  <span title={model}>{model}</span>
                </label>
                {model === defaultModel ? (
                  <span className="n-model-default">默认</span>
                ) : (
                  <button
                    type="button"
                    className="n-inline-action"
                    disabled={disabled}
                    onClick={() => chooseDefault(model)}
                    aria-label={`将 ${model} 设为默认`}
                  >
                    设为默认
                  </button>
                )}
              </div>
            ))}
            {!visible.length && <p className="n-config-note">没有匹配的模型。</p>}
          </div>
        </div>
      )}
      {error && (
        <p className="n-config-note n-error-text" role="alert">
          {error}
        </p>
      )}
      {!error && fetched && (
        <p className="n-config-note">
          已获取 {models.length} 个模型。勾选后点击「保存配置」，默认模型始终包含在所选模型中。
        </p>
      )}
    </div>
  );
}
