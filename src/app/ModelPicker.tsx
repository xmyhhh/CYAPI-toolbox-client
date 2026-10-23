import { useEffect, useRef, useState } from 'react';
import { LoaderCircle, RefreshCw } from 'lucide-react';
import { nativeAvailable, nativeCall } from '../lib/api/native';

type ModelApi = 'openai' | 'anthropic';

interface ModelPickerProps {
  id: string;
  value: string;
  onChange: (value: string) => void;
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
  const request = useRef(0);

  useEffect(() => {
    request.current++;
    setLoading(false);
    setModels([]);
    setError('');
    setFetched(false);
  }, [baseUrl, apiKey, api]);

  const canFetch = nativeAvailable && !disabled && !!baseUrl.trim() && !!apiKey.trim();
  const selected = models.includes(value) ? value : '';

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
        <label htmlFor={id}>模型</label>
        <button
          type="button"
          className="n-inline-action"
          disabled={!canFetch || loading}
          onClick={() => void fetchModels()}
          title="通过 API 的 /models 接口获取模型列表"
        >
          {loading ? (
            <LoaderCircle size={12} className="n-spin" />
          ) : (
            <RefreshCw size={12} />
          )}
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
      />
      {models.length > 0 && (
        <select
          className="n-model-list"
          aria-label="已获取的模型列表"
          value={selected}
          onChange={(event) => {
            if (event.target.value) onChange(event.target.value);
          }}
        >
          <option value="">选择模型（{models.length}）</option>
          {models.map((model) => (
            <option key={model} value={model}>
              {model}
            </option>
          ))}
        </select>
      )}
      {error && (
        <p className="n-config-note n-error-text" role="alert">
          {error}
        </p>
      )}
      {!error && fetched && (
        <p className="n-config-note">
          已获取 {models.length} 个模型，可直接输入其他模型 ID。
        </p>
      )}
    </div>
  );
}
