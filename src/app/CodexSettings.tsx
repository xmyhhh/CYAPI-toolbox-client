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
import type { CodexConfiguration } from '../lib/api/native';
import { CodexSaveConflict, prepareCodexSave } from '../lib/codex-save';
import { RestartPrompt } from '../components/Primitives';
import { CYAPI_DEFAULT_URL, sameRoute } from '../lib/api/cyapi';
import { useCyapiRoutes } from './useCyapiRoutes';
import { CyapiRoutes } from './CyapiRoutes';
import { CyapiBalance } from './CyapiBalance';
import { ModelPicker } from './ModelPicker';
import { ReasoningEffortPicker } from './ReasoningEffortPicker';
import { ContextWindowSettings } from './ContextWindowSettings';
import {
  CodexModelSettings,
  settingDrafts,
  settingsPayload,
  emptySettingDraft,
  sameDraft,
} from './CodexModelSettings';
import type { CodexSettingDraft } from './CodexModelSettings';
import { contextWindowChange, contextWindowText, configuredModels } from '../lib/model-settings';
import { SettingHelp } from './ModelSettingField';

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const preview: CodexConfiguration = {
  configPath: '%CODEX_HOME%\\config.toml',
  authPath: '%CODEX_HOME%\\auth.json',
  revision: '',
  provider: 'openai',
  baseUrl: CYAPI_DEFAULT_URL,
  model: '',
  models: [],
  reasoningEffort: '',
  apiKeyConfigured: false,
  authMode: '网页预览',
  canRestore: false,
};

export function CodexSettings() {
  const [view, setView] = useState<CodexConfiguration>();
  const [baseUrl, setBaseUrl] = useState('');
  const [apiKey, setApiKey] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [model, setModel] = useState('');
  const [models, setModels] = useState<string[]>([]);
  const [effort, setEffort] = useState('');
  const [settings, setSettings] = useState<Record<string, CodexSettingDraft>>({});
  const [contextDraft, setContextDraft] = useState<string>();
  const [busy, setBusy] = useState(true);
  const [notice, setNotice] = useState<{ text: string; error?: boolean; info?: boolean }>();
  const [restartPrompt, setRestartPrompt] = useState(false);
  const [mode, setMode] = useState<'cyapi' | 'custom'>('cyapi');
  const { view: routes, loading: loadingRoutes, refresh: loadRoutes } = useCyapiRoutes();
  const modeTouched = useRef(false);

  function apply(next: CodexConfiguration) {
    const { apiKey: savedKey, ...metadata } = next;
    setView(metadata);
    const empty =
      next.provider === 'openai' &&
      !next.model &&
      !next.apiKeyConfigured &&
      next.authMode !== 'ChatGPT 登录';
    const url = empty ? CYAPI_DEFAULT_URL : next.baseUrl;
    setBaseUrl(url);
    modeTouched.current = false;
    setMode(
      routes.manifest.routes.some((route) => sameRoute(url, route.baseUrl)) ? 'cyapi' : 'custom',
    );
    setModel(next.model);
    setModels(next.models ?? []);
    setEffort(next.reasoningEffort);
    setSettings(settingDrafts(next.modelSettings));
    setContextDraft(undefined);
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
    (nativeAvailable ? nativeCall<CodexConfiguration>('codex-read') : Promise.resolve(preview))
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
  }, []);

  async function reload() {
    setBusy(true);
    setNotice(undefined);
    try {
      apply(await nativeCall<CodexConfiguration>('codex-read'));
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
      const latest = await nativeCall<CodexConfiguration>('codex-read');
      const pending = prepareCodexSave(view, latest, {
        baseUrl,
        apiKey,
        model,
        models,
        modelSettings: settingsPayload(settings, configuredModels(models, model)),
        reasoningEffort: effort,
        contextWindowChange: contextWindowChange(contextDraft),
      });
      apply(
        await nativeCall<CodexConfiguration>('codex-save', {
          codex: pending,
        }),
      );
      setNotice({
        text: '已保存。已有 Codex 会话仍使用开始时读取的 URL；请完全退出并重新启动 Codex，再新建会话使用当前配置。',
      });
      setRestartPrompt(true);
    } catch (error) {
      setNotice({
        text: errorText(error),
        error: !(error instanceof CodexSaveConflict),
        info: error instanceof CodexSaveConflict,
      });
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
        await nativeCall<CodexConfiguration>('codex-restore', {
          configurationRevision: view.revision,
        }),
      );
      setNotice({
        text: '已恢复上次保存前的配置。已有会话仍使用开始时读取的 URL；请完全退出并重新启动 Codex，再新建会话使用恢复后的配置。',
      });
      setRestartPrompt(true);
    } catch (error) {
      setNotice({ text: errorText(error), error: true });
    } finally {
      setBusy(false);
    }
  }

  const routeChanged = Boolean(view && !sameRoute(view.baseUrl, baseUrl));

  return (
    <>
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
            className={`n-alert ${notice.error ? 'n-error' : notice.info ? '' : 'n-success'}`}
            role={notice.error ? 'alert' : 'status'}
          >
            {notice.error || notice.info ? <CircleHelp size={16} /> : <CheckCircle2 size={16} />}
            <span>{notice.text}</span>
          </div>
        )}
        {busy && !view && (
          <p className="n-config-note" role="status">
            正在读取 Codex 配置…
          </p>
        )}
        {view && (
          <p className="n-codex-current">
            <span>{view.provider}</span>
            <span>{view.authMode}</span>
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
                      if (
                        !routes.manifest.routes.some((route) => sameRoute(baseUrl, route.baseUrl))
                      )
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
                  <label htmlFor="codex-api-url">API 地址</label>
                  <input
                    id="codex-api-url"
                    type="url"
                    required
                    value={baseUrl}
                    onChange={(e) => {
                      modeTouched.current = true;
                      setBaseUrl(e.target.value);
                    }}
                    placeholder="https://api.example.com/v1"
                    spellCheck={false}
                  />
                </div>
              )}
              {routeChanged && (
                <p className="n-config-note" role="status">
                  线路已更改，保存后需重启 Codex 并新建会话。
                </p>
              )}
            </section>
            <div className="n-config-field n-api-section">
              <div className="n-config-label-row">
                <label htmlFor="codex-api-key">API Key</label>
                <span className="n-api-section-hint">仅保存在本机</span>
              </div>
              <div className="n-secret-input">
                <input
                  id="codex-api-key"
                  type={showKey ? 'text' : 'password'}
                  required={!view?.apiKeyConfigured}
                  value={apiKey}
                  onChange={(e) => setApiKey(e.target.value)}
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
                id="codex-model"
                value={model}
                onChange={setModel}
                selectedModels={models}
                onModelsChange={setModels}
                baseUrl={baseUrl}
                apiKey={apiKey}
                api="openai"
                disabled={busy || !view}
                required
                placeholder="服务商提供的模型 ID"
                renderModelSummary={(id) => {
                  const draft = settings[id] ?? emptySettingDraft;
                  const original = settingDrafts(view?.modelSettings)[id];
                  return (
                    <>
                      {draft.defaultLevel ? `默认推理 ${draft.defaultLevel}` : '默认推理自动'} ·{' '}
                      {draft.levels.length ? `${draft.levels.length} 个可选等级` : '等级沿用目录'} ·{' '}
                      {draft.contextWindow ? `${draft.contextWindow} tokens` : '上下文沿用目录'}
                      {!sameDraft(original, draft) && (
                        <span className="n-model-pending">待保存</span>
                      )}
                    </>
                  );
                }}
                renderModelSettings={(id, index) => {
                  const originals = settingDrafts(view?.modelSettings);
                  const draft = settings[id] ?? emptySettingDraft;
                  return (
                    <CodexModelSettings
                      id={`codex-permodel-${index}`}
                      model={id}
                      draft={draft}
                      original={originals[id]}
                      onChange={(next) => setSettings((current) => ({ ...current, [id]: next }))}
                      onReset={() =>
                        setSettings((current) => {
                          const next = { ...current };
                          if (originals[id]) next[id] = originals[id];
                          else delete next[id];
                          return next;
                        })
                      }
                      disabled={busy || !view}
                    />
                  );
                }}
              />
              <details className="n-global-settings">
                <summary>
                  <span className="n-model-heading">
                    全局覆盖 <span className="n-model-count">高级</span>
                  </span>
                  <span className="n-global-summary">
                    {effort || contextWindowText(contextDraft ?? view?.contextWindow)
                      ? `已启用${effort ? ` · 推理 ${effort}` : ''}${contextWindowText(contextDraft ?? view?.contextWindow) ? ` · ${contextWindowText(contextDraft ?? view?.contextWindow)} tokens` : ''}`
                      : '未启用 · 使用各模型设置'}
                  </span>
                </summary>
                <div className="n-global-settings-body">
                  <p className="n-global-warning">
                    作用于所有模型，优先于上方逐模型默认值。不确定时请保持“不覆盖”。
                  </p>
                  <div className="n-model-editor-grid">
                    <ReasoningEffortPicker id="codex-effort" value={effort} onChange={setEffort} />
                    <ContextWindowSettings
                      inline
                      id="codex-context"
                      scope="Codex 全局"
                      original={view?.contextWindow}
                      draft={contextDraft}
                      onChange={(value) =>
                        setContextDraft(
                          value === contextWindowText(view?.contextWindow) ? undefined : value,
                        )
                      }
                      onReset={() => setContextDraft(undefined)}
                      disabled={busy || !view}
                    >
                      <p className="n-config-note">
                        写入全局 model_context_window，切换模型后仍适用，不是逐模型能力声明。Profile
                        中的同名字段可能覆盖它；不修改自动压缩阈值或模型目录元数据。
                      </p>
                    </ContextWindowSettings>
                  </div>
                </div>
              </details>
            </section>
          </fieldset>
          <SettingHelp>
            <p className="n-config-note">
              支持 Responses API。保存前自动备份，API Key 仅保存在本机。保存后需完全关闭 Codex
              再重新启动。
            </p>
          </SettingHelp>
          <div className="n-config-savebar">
            <p className="n-config-note">保存前自动备份 · 保存后需重启 Codex</p>
            <div className="n-codex-actions">
              {view?.canRestore && (
                <button
                  type="button"
                  className="n-button"
                  disabled={busy || !nativeAvailable}
                  onClick={restore}
                  title="恢复上次保存前的配置和登录凭据"
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
                {busy ? <LoaderCircle size={15} className="n-spin" /> : <Save size={15} />}保存配置
              </button>
            </div>
          </div>
        </form>
        {view && (
          <details className="n-source-details">
            <summary>配置文件位置</summary>
            <p>
              <code>{view.configPath}</code>
              <br />
              <code>{view.authPath}</code>
            </p>
          </details>
        )}
      </section>
      {restartPrompt && (
        <RestartPrompt target="Codex（包括托盘和后台进程）" onClose={() => setRestartPrompt(false)}>
          <p>已有 Codex 会话会继续使用创建时读取的 URL。</p>
          <p>请完全退出并重新打开 Codex，再新建会话使用当前配置。</p>
        </RestartPrompt>
      )}
    </>
  );
}
