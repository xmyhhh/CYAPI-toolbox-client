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
import { RestartPrompt } from '../components/Primitives';
import {
  CYAPI_DEFAULT_URL,
  cachedCyapiRoutes,
  defaultCyapiRoutes,
  readCyapiRoutes,
  sameRoute,
} from '../lib/api/cyapi';
import type { CyapiRoutesView } from '../lib/api/cyapi';
import { CyapiRoutes } from './CyapiRoutes';
import { CyapiBalance } from './CyapiBalance';
import { ModelPicker } from './ModelPicker';

const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));
const preview: OpenCodeConfiguration = {
  configPath: '%USERPROFILE%\\.config\\opencode\\opencode.json',
  revision: '',
  baseUrl: '',
  model: '',
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
  const [busy, setBusy] = useState(true);
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [restartPrompt, setRestartPrompt] = useState(false);
  const [mode, setMode] = useState<'cyapi' | 'custom'>('cyapi');
  const [routes, setRoutes] = useState<CyapiRoutesView>(
    () => cachedCyapiRoutes() ?? defaultCyapiRoutes,
  );
  const [loadingRoutes, setLoadingRoutes] = useState(() => !cachedCyapiRoutes());
  const modeTouched = useRef(false);
  const routeLoad = useRef(0);

  function apply(next: OpenCodeConfiguration) {
    const { apiKey: savedKey, ...metadata } = next;
    setView(metadata);
    const url = next.baseUrl || CYAPI_DEFAULT_URL;
    setBaseUrl(url);
    modeTouched.current = false;
    setMode(
      routes.manifest.routes.some((route) => sameRoute(url, route.baseUrl))
        ? 'cyapi'
        : 'custom',
    );
    setModel(next.model);
    setApiKey(savedKey ?? '');
    setShowKey(false);
  }

  async function loadRoutes(force = false) {
    const current = ++routeLoad.current;
    setLoadingRoutes(true);
    try {
      const next = await readCyapiRoutes({ force });
      if (routeLoad.current === current) setRoutes(next);
    } catch {
      if (routeLoad.current === current)
        setRoutes({ ...defaultCyapiRoutes, message: '暂时无法更新线路，请稍后重试。' });
    } finally {
      if (routeLoad.current === current) setLoadingRoutes(false);
    }
  }

  useEffect(() => {
    if (!cachedCyapiRoutes()) void loadRoutes();
    return () => {
      routeLoad.current++;
    };
  }, []);

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
    (nativeAvailable ? nativeCall<OpenCodeConfiguration>('opencode-read') : Promise.resolve(preview))
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
          opencode: { baseUrl, apiKey, model, revision: view.revision },
        }),
      );
      setNotice({ text: '已写入 OpenCode 配置。请完全关闭 OpenCode（包括后台进程）后重新启动，配置才会生效。' });
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
        await nativeCall<OpenCodeConfiguration>('opencode-restore', {
          configurationRevision: view.revision,
        }),
      );
      setNotice({ text: '已恢复上次保存前的 OpenCode 配置。请完全关闭 OpenCode 后重新启动。' });
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
          {mode === 'cyapi' ? (
            <CyapiRoutes
              view={routes}
              loading={loadingRoutes}
              disabled={busy || !view}
              value={baseUrl}
              onChange={(url) => {
                modeTouched.current = true;
                setBaseUrl(url);
              }}
              onReload={() => void loadRoutes(true)}
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
          <div className="n-config-field">
            <label htmlFor="opencode-api-key">API Key</label>
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
          <ModelPicker
            id="opencode-model"
            value={model}
            onChange={setModel}
            baseUrl={baseUrl}
            apiKey={apiKey}
            api="openai"
            disabled={busy || !view}
            required
            placeholder="填写服务商提供的模型 ID，如 gpt-5.1-codex"
          />
        </fieldset>
        <p className="n-config-note">
          OpenCode 使用 OpenAI 兼容接口，API 地址通常以 /v1 结尾。保存时保留 JSONC
          注释和其他配置，API Key 仅保存在本机。保存后需完全关闭 OpenCode 再重新启动。
        </p>
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
          target="OpenCode（包括后台进程）"
          onClose={() => setRestartPrompt(false)}
        />
      )}
    </section>
  );
}
