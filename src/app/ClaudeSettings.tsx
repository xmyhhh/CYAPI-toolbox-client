import { useEffect, useState } from 'react';
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
import type { ClaudeConfiguration } from '../lib/api/native';
import {
  CYAPI_DEFAULT_URL,
  cachedCyapiRoutes,
  defaultCyapiRoutes,
  readCyapiRoutes,
  sameRoute,
} from '../lib/api/cyapi';
import type { CyapiRoutesView } from '../lib/api/cyapi';
import { ModelPicker } from './ModelPicker';
import { ModelReasoningSettings } from './ModelReasoningSettings';
import { changeModelEffort, claudeModelEffort, selectedEffortChanges } from '../lib/model-settings';
import { CyapiBalance } from './CyapiBalance';
import { ContextWindowSettings } from './ContextWindowSettings';
import { contextWindowChange, contextWindowText } from '../lib/model-settings';

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

/** Anthropic SDK appends /v1/messages, so the configured base must not end in /v1. */
export function anthropicBase(url: string) {
  return url.trim().replace(/\/+$/, '').replace(/\/v1$/, '');
}

const preview: ClaudeConfiguration = {
  configPath: '%USERPROFILE%\\.claude\\settings.json',
  revision: '',
  baseUrl: '',
  model: '',
  models: [],
  reasoningEffort: '',
  authTokenConfigured: false,
  apiKeyConfigured: false,
  authMode: '网页预览',
  settingsExists: false,
  cliInstalled: false,
  cliPath: '',
  canRestore: false,
};

export function ClaudeSettings() {
  const [view, setView] = useState<ClaudeConfiguration>();
  const [baseUrl, setBaseUrl] = useState(anthropicBase(CYAPI_DEFAULT_URL));
  const [token, setToken] = useState('');
  const [model, setModel] = useState('');
  const [models, setModels] = useState<string[]>([]);
  const [effortChanges, setEffortChanges] = useState<Record<string, string>>({});
  const [clearEffortOverrides, setClearEffortOverrides] = useState(false);
  const [contextDraft, setContextDraft] = useState<string>();
  const [showKey, setShowKey] = useState(false);
  const [busy, setBusy] = useState(true);
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [mode, setMode] = useState<'cyapi' | 'custom'>('cyapi');
  const [routes, setRoutes] = useState<CyapiRoutesView>(
    () => cachedCyapiRoutes() ?? defaultCyapiRoutes,
  );
  const [loadingRoutes, setLoadingRoutes] = useState(() => !cachedCyapiRoutes());

  function apply(next: ClaudeConfiguration) {
    const { authToken: savedToken, ...metadata } = next;
    setView(metadata);
    const url = next.baseUrl ? anthropicBase(next.baseUrl) : anthropicBase(CYAPI_DEFAULT_URL);
    setBaseUrl(url);
    setToken(savedToken ?? '');
    setModel(next.model);
    setModels(next.models ?? []);
    setEffortChanges({});
    setClearEffortOverrides(false);
    setContextDraft(undefined);
    setShowKey(false);
    setMode(
      routes.manifest.routes.some((route) => sameRoute(anthropicBase(route.baseUrl), url))
        ? 'cyapi'
        : 'custom',
    );
  }

  async function loadRoutes(force = false) {
    setLoadingRoutes(true);
    try {
      setRoutes(await readCyapiRoutes({ force }));
    } catch {
      setRoutes({
        ...defaultCyapiRoutes,
        message: '暂时无法更新线路，可使用官方默认线路或稍后重试。',
      });
    } finally {
      setLoadingRoutes(false);
    }
  }

  useEffect(() => {
    if (!cachedCyapiRoutes()) void loadRoutes();
  }, []);

  useEffect(() => {
    let active = true;
    (nativeAvailable ? nativeCall<ClaudeConfiguration>('claude-read') : Promise.resolve(preview))
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

  const routeOptions = routes.manifest.routes.map((route) => ({
    ...route,
    base: anthropicBase(route.baseUrl),
  }));
  const selectedRoute = routeOptions.find((route) => sameRoute(route.base, baseUrl));

  async function reload() {
    setBusy(true);
    setNotice(undefined);
    try {
      apply(await nativeCall<ClaudeConfiguration>('claude-read'));
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
      const next = await nativeCall<ClaudeConfiguration>('claude-save', {
        claude: {
          baseUrl,
          authToken: token,
          model,
          models,
          modelEffortChanges: selectedEffortChanges(effortChanges, models, model),
          clearEffortOverrides,
          contextWindowChange: contextWindowChange(contextDraft),
          revision: view.revision,
        },
      });
      apply(next);
      setNotice({
        text: '已写入 Claude Code 配置。请启动新会话使用新的接入设置；当前会话的模型与等级可用 /model、/effort 切换。',
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
        await nativeCall<ClaudeConfiguration>('claude-restore', {
          configurationRevision: view.revision,
        }),
      );
      setNotice({
        text: '已恢复上次保存前的 settings.json。请启动新会话使用恢复后的接入设置。',
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
          正在读取 Claude Code 配置…
        </p>
      )}
      {view && (
        <p className="n-codex-current">
          <span>{view.settingsExists ? 'settings.json' : '未创建 settings.json'}</span>
          <span>{view.authMode}</span>
          <span title={view.cliPath || undefined}>
            {view.cliInstalled ? '已检测到 claude 命令' : '未在 PATH 中找到 claude'}
          </span>
        </p>
      )}
      <form onSubmit={save} autoComplete="off">
        <fieldset disabled={busy || !view} className="n-codex-fields">
          <div className="n-cyapi-mode" role="group" aria-label="API 服务类型">
            <button
              type="button"
              aria-pressed={mode === 'cyapi'}
              onClick={() => {
                setMode('cyapi');
                if (!selectedRoute)
                  setBaseUrl(routeOptions[0]?.base ?? anthropicBase(CYAPI_DEFAULT_URL));
              }}
            >
              CYAPI 专属
            </button>
            <button
              type="button"
              aria-pressed={mode === 'custom'}
              onClick={() => {
                setMode('custom');
              }}
            >
              自定义服务
            </button>
          </div>
          {mode === 'cyapi' ? (
            <div className="n-config-field">
              <div className="n-config-label-row">
                <label htmlFor="claude-route">线路</label>
                <button
                  type="button"
                  className="n-inline-action"
                  disabled={loadingRoutes}
                  onClick={() => void loadRoutes(true)}
                >
                  <RefreshCw size={12} className={loadingRoutes ? 'n-spin' : ''} />
                  {loadingRoutes ? '正在更新' : '更新线路'}
                </button>
              </div>
              <select
                id="claude-route"
                value={selectedRoute?.id ?? ''}
                onChange={(event) => {
                  const route = routeOptions.find((item) => item.id === event.target.value);
                  if (route) {
                    setBaseUrl(route.base);
                  }
                }}
              >
                {!selectedRoute && <option value="">自定义地址</option>}
                {routeOptions.map((route) => (
                  <option key={route.id} value={route.id}>
                    {route.name}
                  </option>
                ))}
              </select>
              <p className="n-config-note">
                <code>{baseUrl || anthropicBase(CYAPI_DEFAULT_URL)}</code>
                {loadingRoutes ? ' · 正在更新线路' : ''}
              </p>
            </div>
          ) : (
            <div className="n-config-field">
              <label htmlFor="claude-api-url">API 地址</label>
              <input
                id="claude-api-url"
                type="url"
                required
                value={baseUrl}
                onChange={(event) => {
                  setBaseUrl(event.target.value);
                }}
                placeholder="https://api.example.com"
                spellCheck={false}
              />
            </div>
          )}
          <div className="n-config-field">
            <label htmlFor="claude-auth-token">AUTH_TOKEN</label>
            <div className="n-secret-input">
              <input
                id="claude-auth-token"
                type={showKey ? 'text' : 'password'}
                required={!view?.authTokenConfigured}
                value={token}
                onChange={(event) => setToken(event.target.value)}
                placeholder={view?.authTokenConfigured ? '留空保留原 Token' : '填写 AUTH_TOKEN'}
                autoComplete="new-password"
                spellCheck={false}
              />
              <button
                type="button"
                className="n-icon-button"
                onClick={() => setShowKey(!showKey)}
                disabled={!token}
                aria-label={showKey ? '隐藏 AUTH_TOKEN' : '显示 AUTH_TOKEN'}
                aria-pressed={showKey}
                title={showKey ? '隐藏 AUTH_TOKEN' : '显示 AUTH_TOKEN'}
              >
                {showKey ? <EyeOff size={17} /> : <Eye size={17} />}
              </button>
            </div>
          </div>
          {mode === 'cyapi' && (
            <CyapiBalance
              baseUrl={selectedRoute?.baseUrl ?? baseUrl}
              apiKey={token}
              disabled={busy || !view}
            />
          )}
          {(view?.effortOverrides?.environment || view?.effortOverrides?.global) && (
            <div className="n-model-override" role="status">
              <strong>检测到统一推理等级设置</strong>
              {view.effortOverrides.environment && (
                <p>
                  <code>env.CLAUDE_CODE_EFFORT_LEVEL = {view.effortOverrides.environment}</code>
                  （会覆盖逐模型设置）
                </p>
              )}
              {view.effortOverrides.global && (
                <p>
                  <code>effortLevel = {view.effortOverrides.global}</code>（适用模型的全局回退值）
                </p>
              )}
              <p>
                默认保留这两项。切换为逐模型配置后，保存时将删除本文件中的上述设置，未单独设置的模型回到其他配置或工具默认；不会自动复制统一等级。
              </p>
              <button
                type="button"
                className="n-button"
                disabled={busy || !view}
                aria-pressed={clearEffortOverrides}
                onClick={() => setClearEffortOverrides(!clearEffortOverrides)}
              >
                {clearEffortOverrides ? '撤销切换，保留统一设置' : '切换为逐模型配置'}
              </button>
              {clearEffortOverrides && (
                <p>待保存：将移除上述统一设置。Shell 环境变量或项目配置仍可能覆盖本文件。</p>
              )}
            </div>
          )}
          <ModelPicker
            id="claude-model"
            value={model}
            onChange={setModel}
            selectedModels={models}
            onModelsChange={setModels}
            baseUrl={baseUrl}
            apiKey={token}
            api="anthropic"
            disabled={busy || !view}
            placeholder="输入完整模型 ID；默认模型可选择由 Claude Code 决定"
            renderModelSettings={(id, index) => {
              const original = claudeModelEffort(
                id,
                view?.modelEfforts ?? {},
                view?.modelEffortKeys,
              );
              return (
                <ModelReasoningSettings
                  id={`claude-effort-${index}`}
                  model={id}
                  original={original}
                  value={Object.hasOwn(effortChanges, id) ? effortChanges[id] : original}
                  onChange={(value) =>
                    setEffortChanges((changes) => changeModelEffort(changes, id, value, original))
                  }
                  api="anthropic"
                  disabled={busy || !view}
                />
              );
            }}
          />
          <ContextWindowSettings
            id="claude-context"
            scope="Claude Code 全局"
            original={view?.contextWindow}
            draft={contextDraft}
            onChange={(value) =>
              setContextDraft(value === contextWindowText(view?.contextWindow) ? undefined : value)
            }
            onReset={() => setContextDraft(undefined)}
            disabled={busy || !view}
          >
            <p className="n-config-note">
              高级覆盖：写入
              settings.env.CLAUDE_CODE_MAX_CONTEXT_TOKENS，不是逐模型设置，也不保证对当前 ID
              生效。请重新启动 Claude Code 并新建会话核对；Shell 环境和项目配置可能覆盖本文件。
            </p>
            <p className="n-config-note">
              通常仅未识别、不以 claude- 开头且无 [1m] 的自定义 ID 可直接生效。已识别模型或裸
              claude- ID 还需 DISABLE_COMPACT（会关闭全部压缩）；未识别的 [1m] ID 还涉及
              CLAUDE_CODE_DISABLE_1M_CONTEXT。工具箱不会自动修改这些开关、模型 ID 或压缩阈值。
            </p>
          </ContextWindowSettings>
          {view?.apiKeyConfigured && (
            <p className="n-config-note">
              settings.json 里还设置了 ANTHROPIC_API_KEY；如果请求鉴权异常，请只保留 AUTH_TOKEN 或
              API Key 其中一个。
            </p>
          )}
        </fieldset>
        <p className="n-config-note">
          Anthropic 接口地址末尾的 /v1 会自动去掉。保存前自动备份 settings.json，AUTH_TOKEN
          仅保存在本机。保存后请启动新会话；当前会话的模型与等级可用 /model、/effort 切换。
        </p>
        <div className="n-codex-actions">
          {view?.canRestore && (
            <button
              type="button"
              className="n-button"
              disabled={busy || !nativeAvailable}
              onClick={restore}
              title="恢复上次保存前的 Claude Code 配置"
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
