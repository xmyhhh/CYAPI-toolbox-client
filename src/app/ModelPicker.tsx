import { useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { ChevronDown, LoaderCircle, Plus, RefreshCw, X } from 'lucide-react';
import { nativeAvailable, nativeCall } from '../lib/api/native';
import { configuredModels } from '../lib/model-settings';

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
  renderModelSettings?: (model: string, index: number) => ReactNode;
  renderModelSummary?: (model: string) => ReactNode;
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
  renderModelSettings,
  renderModelSummary,
}: ModelPickerProps) {
  const [models, setModels] = useState<string[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [fetched, setFetched] = useState(false);
  const [adding, setAdding] = useState(false);
  const [manual, setManual] = useState('');
  const [search, setSearch] = useState('');
  const [pending, setPending] = useState<string[]>([]);
  const [editingModel, setEditingModel] = useState<string>();
  const request = useRef(0);

  useEffect(() => {
    request.current++;
    setLoading(false);
    setModels([]);
    setError('');
    setFetched(false);
    setSearch('');
    setPending([]);
    return () => {
      request.current++;
    };
  }, [baseUrl, apiKey, api]);

  const canFetch = nativeAvailable && !disabled && !!baseUrl.trim() && !!apiKey.trim();
  const defaultModel = value.trim();
  const selected = configuredModels(selectedModels, defaultModel);
  const visible = models.filter(
    (model) =>
      !selected.includes(model) && model.toLowerCase().includes(search.trim().toLowerCase()),
  );

  function chooseDefault(model: string) {
    onModelsChange(configuredModels(selected, model));
    onChange(model);
  }

  function add(ids: string[]) {
    if (
      ids.some(
        (id) =>
          !id.trim() ||
          new TextEncoder().encode(id).length > 200 ||
          /[\u0000-\u001f\u007f-\u009f]/u.test(id),
      )
    ) {
      setError('模型 ID 不能为空、包含控制字符或超过 200 字节。');
      return false;
    }
    const next = configuredModels([...selected, ...ids], defaultModel);
    if (next.length > 200) {
      setError('最多配置 200 个模型，请先移除不需要的模型。');
      return false;
    }
    setError('');
    onModelsChange(next);
    if (!defaultModel && required && next.length) onChange(next[0]);
    return true;
  }

  function addManual() {
    const model = manual.trim();
    if (!model) return;
    if (add([model])) setManual('');
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
      setModels(configuredModels(result.models, ''));
      setPending([]);
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
        <span className="n-model-heading">
          模型 <span className="n-model-count">{selected.length}</span>
        </span>
        <button
          type="button"
          className="n-inline-action"
          disabled={disabled}
          aria-expanded={adding}
          aria-controls={`${id}-add`}
          onClick={() => setAdding(!adding)}
        >
          {adding ? <X size={12} /> : <Plus size={12} />}
          {adding ? '收起添加' : '添加模型'}
        </button>
      </div>
      <div className="n-model-default-picker">
        <label htmlFor={id}>默认模型</label>
        <select
          id={id}
          required={required}
          value={defaultModel}
          onChange={(event) => chooseDefault(event.target.value)}
          disabled={disabled}
        >
          <option value="" disabled={required}>
            {required ? '请先添加并选择模型' : '使用工具自身默认模型'}
          </option>
          {selected.map((model) => (
            <option key={model} value={model}>
              {model}
            </option>
          ))}
        </select>
      </div>
      <p className="n-config-note n-model-caption">用于工具内切换，修改后统一保存。</p>
      {selected.length > 0 ? (
        <div className="n-model-list" role="group" aria-label="已配置模型">
          {selected.map((model, index) => (
            <div
              key={model}
              className={`n-model-configured${editingModel === model ? ' is-editing' : ''}`}
            >
              <div className="n-model-choice">
                {renderModelSettings ? (
                  <button
                    type="button"
                    className="n-model-edit-toggle"
                    disabled={disabled}
                    aria-expanded={editingModel === model}
                    aria-controls={`${id}-settings-${index}`}
                    aria-label={`配置 ${model}`}
                    onClick={() => setEditingModel(editingModel === model ? undefined : model)}
                  >
                    <ChevronDown
                      size={15}
                      className={editingModel === model ? 'n-chevron-open' : ''}
                    />
                    <span className="n-model-identity">
                      <span className="n-model-id">{model}</span>
                      {renderModelSummary && (
                        <span className="n-model-overview">{renderModelSummary(model)}</span>
                      )}
                    </span>
                    <span className="n-model-edit-label">
                      {editingModel === model ? '收起' : '设置'}
                    </span>
                  </button>
                ) : (
                  <span className="n-model-id" title={model}>
                    {model}
                  </span>
                )}
                {model === defaultModel && <span className="n-model-default">默认</span>}
                <button
                  type="button"
                  className="n-inline-action"
                  disabled={disabled || model === defaultModel}
                  title={model === defaultModel ? '请先选择另一个默认模型再移除' : `移除 ${model}`}
                  aria-label={`移除 ${model}`}
                  onClick={() => {
                    onModelsChange(selected.filter((id) => id !== model));
                    if (editingModel === model) setEditingModel(undefined);
                  }}
                >
                  移除
                </button>
              </div>
              {renderModelSettings && (
                <div
                  id={`${id}-settings-${index}`}
                  hidden={editingModel !== model}
                  className="n-model-settings"
                >
                  {editingModel === model && renderModelSettings(model, index)}
                </div>
              )}
            </div>
          ))}
        </div>
      ) : (
        <p className="n-config-note">尚未添加模型。可从接口获取，也可手动添加模型 ID。</p>
      )}
      {adding && (
        <div className="n-model-add" id={`${id}-add`}>
          <div className="n-config-label-row">
            <span className="n-model-heading">添加模型</span>
            <button
              type="button"
              className="n-inline-action"
              disabled={!canFetch || loading}
              onClick={() => void fetchModels()}
              title="通过当前 API 的模型列表接口获取，不验证推理能力"
            >
              {loading ? <LoaderCircle size={12} className="n-spin" /> : <RefreshCw size={12} />}
              {loading ? '正在获取' : '获取模型列表'}
            </button>
          </div>
          <div className="n-model-manual">
            <input
              aria-label="手动添加模型 ID"
              value={manual}
              onChange={(event) => setManual(event.target.value)}
              onKeyDown={(event) => {
                if (event.key === 'Enter') {
                  event.preventDefault();
                  addManual();
                }
              }}
              placeholder={placeholder || '输入服务商提供的模型 ID'}
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
          {!canFetch && (
            <p className="n-config-note">
              在桌面端填写 API 地址和 Key 后可获取列表；也可以直接手动添加。
            </p>
          )}
          {fetched && (
            <>
              <input
                aria-label="搜索待添加模型"
                placeholder="搜索模型 ID"
                value={search}
                onChange={(event) => setSearch(event.target.value)}
                disabled={disabled}
              />
              <div className="n-model-list" role="group" aria-label="待添加模型">
                {visible.map((model) => (
                  <div key={model} className="n-model-choice">
                    <label>
                      <input
                        type="checkbox"
                        checked={pending.includes(model)}
                        disabled={disabled}
                        onChange={(event) =>
                          setPending(
                            event.target.checked
                              ? [...pending, model]
                              : pending.filter((id) => id !== model),
                          )
                        }
                      />
                      <span>{model}</span>
                    </label>
                  </div>
                ))}
                {!visible.length && <p className="n-config-note">没有待添加的匹配模型。</p>}
              </div>
              <button
                type="button"
                className="n-button"
                disabled={disabled || !pending.length}
                onClick={() => {
                  if (add(pending)) setPending([]);
                }}
              >
                添加所选 {pending.length > 0 ? `${pending.length} 个模型` : '模型'}
              </button>
              <p className="n-config-note">已获取 {models.length} 个模型，未验证接口及推理能力。</p>
            </>
          )}
          {error && (
            <p className="n-config-note n-error-text" role="alert">
              {error}
            </p>
          )}
        </div>
      )}
    </div>
  );
}
