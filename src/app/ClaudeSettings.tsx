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
import { RestartPrompt } from '../components/Primitives';
import {
  CYAPI_DEFAULT_URL,
  cachedCyapiRoutes,
  defaultCyapiRoutes,
  readCyapiRoutes,
  sameRoute,
} from '../lib/api/cyapi';
import type { CyapiRoutesView } from '../lib/api/cyapi';
import { ModelPicker } from './ModelPicker';
import { ReasoningEffortPicker } from './ReasoningEffortPicker';
import { CyapiBalance } from './CyapiBalance';

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
  const [effort, setEffort] = useState('');
  const [showKey, setShowKey] = useState(false);
  const [busy, setBusy] = useState(true);
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [restartPrompt, setRestartPrompt] = useState(false);
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
    setEffort(next.reasoningEffort ?? '');
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
          reasoningEffort: effort,
          revision: view.revision,
        },
      });
      apply(next);
      setNotice({
        text: '已写入 Claude Code 配置。请完全关闭当前 Claude Code 会话和后台进程，重新启动后生效。',
      });
      setRestartPrompt(true);
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
        text: '已恢复上次保存前的 settings.json。请完全关闭当前 Claude Code 会话后重新启动。',
      });
      setRestartPrompt(true);
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
            placeholder="留空使用 Claude Code 默认模型"
          />
          <ReasoningEffortPicker
            id="claude-effort"
            value={effort}
            onChange={setEffort}
            api="anthropic"
          />
          {view?.apiKeyConfigured && (
            <p className="n-config-note">
              settings.json 里还设置了 ANTHROPIC_API_KEY；如果请求鉴权异常，请只保留 AUTH_TOKEN 或
              API Key 其中一个。
            </p>
          )}
        </fieldset>
        <p className="n-config-note">
          Anthropic 接口地址末尾的 /v1 会自动去掉。保存前自动备份 settings.json，AUTH_TOKEN
          仅保存在本机。保存后需完全关闭当前 Claude Code 会话和后台进程再重新启动。
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
      {restartPrompt && (
        <RestartPrompt
          target="Claude Code（包括当前会话和后台进程）"
          onClose={() => setRestartPrompt(false)}
        />
      )}
    </section>
  );
}
