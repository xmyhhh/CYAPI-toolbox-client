import { useEffect, useRef, useState } from 'react';
import {
  CheckCircle2,
  CircleHelp,
  Eye,
  EyeOff,
  LoaderCircle,
  RefreshCw,
  RotateCcw,
  Save,
} from 'lucide-react';
import { nativeAvailable, nativeCall } from '../lib/api/native';
import type { OpenCodeConfiguration } from '../lib/api/native';
import {
  changeModelEffort,
  contextWindowText,
  modelEffort,
  selectedEffortChanges,
  selectedContextChanges,
  selectedReasoningLevelChanges,
  reasoningLevelsText,
} from '../lib/model-settings';
import { ContextWindowSettings } from './ContextWindowSettings';
import { CYAPI_DEFAULT_URL, sameRoute } from '../lib/api/cyapi';
import { useCyapiRoutes } from './useCyapiRoutes';
import { CyapiRoutes } from './CyapiRoutes';
import { CyapiBalance } from './CyapiBalance';
import { ModelPicker } from './ModelPicker';
import { ModelReasoningSettings } from './ModelReasoningSettings';
import { OpenCodeReasoningLevels } from './OpenCodeReasoningLevels';
import { SettingHelp } from './ModelSettingField';

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const preview: OpenCodeConfiguration = {
  configPath: '%USERPROFILE%\\.config\\opencode\\opencode.json',
  revision: '',
  baseUrl: '',
  model: '',
  models: [],
  reasoningEffort: '',
  apiKeyConfigured: false,
  authMode: '网页预览',
  settingsExists: false,
  cliInstalled: false,
  cliPath: '',
  canRestore: false,
};

export function OpenCodeSettings() {
  const [view, setView] = useState<OpenCodeConfiguration>();
  const [baseUrl, setBaseUrl] = useState(CYAPI_DEFAULT_URL);
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [model, setModel] = useState('');
  const [models, setModels] = useState<string[]>([]);
  const [effortChanges, setEffortChanges] = useState<Record<string, string>>({});
  const [contextChanges, setContextChanges] = useState<Record<string, string>>({});
  const [levelChanges, setLevelChanges] = useState<Record<string, string>>({});
  const [busy, setBusy] = useState(true);
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [mode, setMode] = useState<'cyapi' | 'custom'>('cyapi');
  const { view: routes, loading: loadingRoutes, refresh: loadRoutes } = useCyapiRoutes();
  const modeTouched = useRef(false);

  function apply(next: OpenCodeConfiguration) {
    const { apiKey: savedKey, ...metadata } = next;
    setView(metadata);
    const url = next.baseUrl || CYAPI_DEFAULT_URL;
    setBaseUrl(url);
    modeTouched.current = false;
    setMode(
      routes.manifest.routes.some((route) => sameRoute(url, route.baseUrl)) ? 'cyapi' : 'custom',
    );
    setModel(next.model);
    setModels(next.models ?? []);
    setEffortChanges({});
    setContextChanges({});
    setLevelChanges({});
    setApiKey(savedKey ?? '');
    setShowKey(false);
  }

  useEffect(() => {
    if (!modeTouched.current && view) {
      setMode(
        routes.manifest.routes.some((route) => sameRoute(baseUrl, route.baseUrl))
          ? 'cyapi'
          : 'custom',
      );
    }
  }, [routes, baseUrl, view]);

  useEffect(() => {
    let active = true;
    (nativeAvailable
      ? nativeCall<OpenCodeConfiguration>('opencode-read')
      : Promise.resolve(preview)
    )
      .then((next) => {
        if (active) apply(next);
      })
      .catch((error) => {
        if (active) setNotice({ text: errorText(error), error: true });
      })
      .finally(() => {
        if (active) setBusy(false);
      });
    return () => {
      active = false;
    };
    // Apply only on mount; later updates go through reload/save.
  }, []);

  async function reload() {
    setBusy(true);
    setNotice(undefined);
    try {
      apply(await nativeCall<OpenCodeConfiguration>('opencode-read'));
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
    } finally {
      setBusy(false);
    }
  }

  async function save(event: React.FormEvent) {
    event.preventDefault();
    if (!view) return;
    setBusy(true);
    setNotice(undefined);
    try {
      apply(
        await nativeCall<OpenCodeConfiguration>('opencode-save', {
          opencode: {
            baseUrl,
            apiKey,
            model,
            models,
            modelEffortChanges: selectedEffortChanges(effortChanges, models, model),
            modelContextChanges: selectedContextChanges(contextChanges, models, model),
            modelReasoningLevelChanges: selectedReasoningLevelChanges(levelChanges, models, model),
            revision: view.revision,
          },
        }),
      );
      setNotice({
        text: '已写入 OpenCode 配置。OpenCode V2 会自动重载；当前会话的模型或 variant 选择仍优先生效，可在 /models 中切换。',
      });
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
    } finally {
      setBusy(false);
    }
  }

  async function restore() {
    if (!view) return;
    setBusy(true);
    setNotice(undefined);
    try {
      apply(
        await nativeCall<OpenCodeConfiguration>('opencode-restore', {
          configurationRevision: view.revision,
        }),
      );
      setNotice({
        text: '已恢复上次保存前的 OpenCode 配置。OpenCode V2 会自动重载，也可使用 /reload。',
      });
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="n-panel n-codex-panel">
      <div className="n-panel-heading">
        <h2>API 配置</h2>
        <button className="n-button" disabled={busy || !nativeAvailable} onClick={reload}>
          <RefreshCw size={14} className={busy ? 'n-spin' : ''} />
          重新读取
        </button>
      </div>
      {notice && (
        <div
          className={`n-alert ${notice.error ? 'n-error' : 'n-success'}`}
          role={notice.error ? 'alert' : 'status'}
        >
          {notice.error ? <CircleHelp size={16} /> : <CheckCircle2 size={16} />}
          <span>{notice.text}</span>
        </div>
      )}
      {busy && !view && (
        <p className="n-config-note" role="status">
          正在读取 OpenCode 配置…
        </p>
      )}
      {view && (
        <p className="n-codex-current">
          <span>{view.settingsExists ? 'opencode.json(c)' : '未创建 OpenCode 配置'}</span>
          <span>{view.authMode}</span>
          <span title={view.cliPath || undefined}>
            {view.cliInstalled ? '已检测到 opencode 命令' : '未在 PATH 中找到 opencode'}
          </span>
        </p>
      )}
      <form onSubmit={save} autoComplete="off">
        <fieldset disabled={busy || !view} className="n-codex-fields">
          <section className="n-api-section n-api-route" aria-label="线路">
            <div className="n-api-section-heading">
              <h3>线路</h3>
              <div className="n-cyapi-mode" role="group" aria-label="API 服务类型">
                <button
                  type="button"
                  aria-pressed={mode === 'cyapi'}
                  onClick={() => {
                    modeTouched.current = true;
                    setMode('cyapi');
                    if (!routes.manifest.routes.some((route) => sameRoute(baseUrl, route.baseUrl)))
                      setBaseUrl(CYAPI_DEFAULT_URL);
                  }}
                >
                  CYAPI 专属
                </button>
                <button
                  type="button"
                  aria-pressed={mode === 'custom'}
                  onClick={() => {
                    modeTouched.current = true;
                    setMode('custom');
                  }}
                >
                  自定义服务
                </button>
              </div>
            </div>
            {mode === 'cyapi' ? (
              <CyapiRoutes
                embedded
                view={routes}
                loading={loadingRoutes}
                disabled={busy || !view}
                value={baseUrl}
                onChange={(url) => {
                  modeTouched.current = true;
                  setBaseUrl(url);
                }}
                onReload={() => void loadRoutes()}
              />
            ) : (
              <div className="n-config-field">
                <label htmlFor="opencode-api-url">API 地址</label>
                <input
                  id="opencode-api-url"
                  type="url"
                  required
                  value={baseUrl}
                  onChange={(event) => {
                    modeTouched.current = true;
                    setBaseUrl(event.target.value);
                  }}
                  placeholder="https://api.example.com/v1"
                  spellCheck={false}
                />
              </div>
            )}
          </section>
          <div className="n-config-field n-api-section">
            <div className="n-config-label-row">
              <label htmlFor="opencode-api-key">API Key</label>
              <span className="n-api-section-hint">仅保存在本机</span>
            </div>
            <div className="n-secret-input">
              <input
                id="opencode-api-key"
                type={showKey ? 'text' : 'password'}
                required={!view?.apiKeyConfigured}
                value={apiKey}
                onChange={(event) => setApiKey(event.target.value)}
                placeholder={view?.apiKeyConfigured ? '留空保留原 Key' : '填写 API Key'}
                autoComplete="new-password"
                spellCheck={false}
              />
              <button
                type="button"
                className="n-icon-button"
                onClick={() => setShowKey(!showKey)}
                disabled={!apiKey}
                aria-label={showKey ? '隐藏 API Key' : '显示 API Key'}
                aria-pressed={showKey}
                title={showKey ? '隐藏 API Key' : '显示 API Key'}
              >
                {showKey ? <EyeOff size={17} /> : <Eye size={17} />}
              </button>
            </div>
          </div>
          {mode === 'cyapi' && (
            <CyapiBalance baseUrl={baseUrl} apiKey={apiKey} disabled={busy || !view} />
          )}
          <section className="n-api-section n-api-models" aria-label="模型">
            <ModelPicker
              id="opencode-model"
              value={model}
              onChange={setModel}
              selectedModels={models}
              onModelsChange={setModels}
              baseUrl={baseUrl}
              apiKey={apiKey}
              api="openai"
              disabled={busy || !view}
              required
              placeholder="填写服务商提供的完整模型 ID"
              renderModelSummary={(id) => {
                const effort = Object.hasOwn(effortChanges, id)
                  ? effortChanges[id]
                  : modelEffort(id, view?.modelEfforts ?? {});
                const context = Object.hasOwn(contextChanges, id)
                  ? contextChanges[id]
                  : contextWindowText(view?.modelContexts?.[id]);
                const levels = Object.hasOwn(levelChanges, id)
                  ? levelChanges[id]
                  : reasoningLevelsText(view?.modelReasoningLevels?.[id]);
                const changed = [effortChanges, contextChanges, levelChanges].some((changes) =>
                  Object.hasOwn(changes, id),
                );
                const conflict = [
                  view?.modelEffortConflicts,
                  view?.modelContextConflicts,
                  view?.modelReasoningLevelConflicts,
                ].some((ids) => ids?.includes(id));
                return (
                  <>
                    {effort ? `推理 ${effort}` : '推理不覆盖'} ·{' '}
                    {levels ? `${levels.split(',').length} 个可选等级` : '等级沿用目录'} ·{' '}
                    {context ? `${context} tokens` : '上下文沿用原值'}
                    {conflict ? ' · 存在别名差异' : ''}
                    {changed && <span className="n-model-pending">待保存</span>}
                  </>
                );
              }}
              renderModelSettings={(id, index) => {
                const original = modelEffort(id, view?.modelEfforts ?? {});
                const conflict = view?.modelEffortConflicts?.includes(id) ?? false;
                const contextOriginal = Object.hasOwn(view?.modelContexts ?? {}, id)
                  ? view?.modelContexts?.[id]
                  : null;
                const contextConflict = view?.modelContextConflicts?.includes(id) ?? false;
                return (
                  <div className="n-opencode-model-editor">
                    <div className="n-model-editor-grid">
                      <ModelReasoningSettings
                        inline
                        id={`opencode-effort-${index}`}
                        model={id}
                        original={original}
                        value={Object.hasOwn(effortChanges, id) ? effortChanges[id] : original}
                        conflict={conflict}
                        changed={Object.hasOwn(effortChanges, id)}
                        onChange={(value) =>
                          setEffortChanges((changes) =>
                            changeModelEffort(changes, id, value, original, conflict),
                          )
                        }
                        onReset={() =>
                          setEffortChanges((changes) => {
                            const next = { ...changes };
                            delete next[id];
                            return next;
                          })
                        }
                        api="openai"
                        disabled={busy || !view}
                      />
                      <ContextWindowSettings
                        inline
                        id={`opencode-context-${index}`}
                        scope={`${id} 的`}
                        original={contextOriginal}
                        draft={Object.hasOwn(contextChanges, id) ? contextChanges[id] : undefined}
                        conflict={contextConflict}
                        onChange={(value) =>
                          setContextChanges((changes) =>
                            changeModelEffort(
                              changes,
                              id,
                              value,
                              contextWindowText(contextOriginal),
                              contextConflict,
                            ),
                          )
                        }
                        onReset={() =>
                          setContextChanges((changes) => {
                            const next = { ...changes };
                            delete next[id];
                            return next;
                          })
                        }
                        disabled={busy || !view}
                      />
                    </div>
                    <OpenCodeReasoningLevels
                      inline
                      id={`opencode-levels-${index}`}
                      model={id}
                      original={view?.modelReasoningLevels?.[id]}
                      draft={Object.hasOwn(levelChanges, id) ? levelChanges[id] : undefined}
                      conflict={view?.modelReasoningLevelConflicts?.includes(id) ?? false}
                      editable={view?.reasoningLevelsEditable ?? false}
                      disabled={busy || !view}
                      onChange={(text) =>
                        setLevelChanges((changes) => ({ ...changes, [id]: text }))
                      }
                      onReset={() =>
                        setLevelChanges((changes) => {
                          const next = { ...changes };
                          delete next[id];
                          return next;
                        })
                      }
                    />
                  </div>
                );
              }}
            />
          </section>
        </fieldset>
        <SettingHelp>
          <p className="n-config-note">
            OpenCode 使用 OpenAI 兼容接口，API 地址通常以 /v1 结尾。保存时保留 JSONC
            注释和其他配置，API Key 仅保存在本机。仅修改明确编辑的模型等级和上下文窗口；OpenCode V2
            自动重载配置，必要时使用 /reload。
          </p>
        </SettingHelp>
        <div className="n-config-savebar">
          <p className="n-config-note">更改保存后生效 · OpenCode V2 自动重载</p>
          <div className="n-codex-actions">
            {view?.canRestore && (
              <button
                type="button"
                className="n-button"
                disabled={busy || !nativeAvailable}
                onClick={restore}
                title="恢复上次保存前的 OpenCode 配置"
              >
                <RotateCcw size={14} />
                恢复上次配置
              </button>
            )}
            <button
              className="n-button n-primary"
              type="submit"
              disabled={busy || !nativeAvailable || !view}
            >
              {busy ? <LoaderCircle size={15} className="n-spin" /> : <Save size={15} />}
              保存配置
            </button>
          </div>
        </div>
      </form>
      {view && (
        <details className="n-source-details">
          <summary>配置文件位置</summary>
          <p>
            <code>{view.configPath}</code>
            {view.cliPath && (
              <>
                <br />
                <code>{view.cliPath}</code>
              </>
            )}
          </p>
        </details>
      )}
    </section>
  );
}
