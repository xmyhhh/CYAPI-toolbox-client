import { useEffect, useRef, useState } from 'react';
import { Gauge, LoaderCircle, RefreshCw } from 'lucide-react';
import { nativeAvailable } from '../lib/api/native';
import { probeCyapiRoute, sameRoute } from '../lib/api/cyapi';
import type { CyapiMeasurement, CyapiRoutesView } from '../lib/api/cyapi';
import { SettingHelp } from './ModelSettingField';

interface Props {
  view: CyapiRoutesView;
  loading: boolean;
  disabled: boolean;
  value: string;
  onChange: (value: string) => void;
  onReload: () => void;
  embedded?: boolean;
}
export function CyapiRoutes({
  view,
  loading,
  disabled,
  value,
  onChange,
  onReload,
  embedded,
}: Props) {
  const [results, setResults] = useState<Record<string, CyapiMeasurement>>({});
  const [running, setRunning] = useState(false);
  const [pending, setPending] = useState<string[]>([]);
  const run = useRef(0);
  useEffect(() => {
    run.current++;
    setResults({});
    setPending([]);
    setRunning(false);
    return () => {
      run.current++;
    };
  }, [view]);

  async function testRoutes() {
    const current = ++run.current;
    setResults({});
    setRunning(true);
    const queue = [...view.manifest.routes];
    async function worker() {
      while (queue.length && run.current === current) {
        const route = queue.shift()!;
        setPending((ids) => [...ids, route.id]);
        let result: CyapiMeasurement;
        try {
          result = await probeCyapiRoute(route.id);
        } catch (error) {
          result = {
            routeId: route.id,
            latencyMs: null,
            reachable: false,
            message: error instanceof Error ? error.message : String(error),
          };
        }
        if (run.current !== current) return;
        setResults((previous) => ({ ...previous, [route.id]: result }));
        setPending((ids) => ids.filter((id) => id !== route.id));
      }
    }
    await Promise.all(Array.from({ length: Math.min(4, queue.length) }, worker));
    if (run.current === current) setRunning(false);
  }
  const successful = Object.values(results).filter((r) => r.reachable && r.latencyMs !== null);
  const best = successful.length ? Math.min(...successful.map((r) => r.latencyMs!)) : null;
  const selected = view.manifest.routes.some((route) => sameRoute(value, route.baseUrl));
  const completed = Object.keys(results).length;
  return (
    <section
      className={`n-cyapi-routes${embedded ? ' n-cyapi-routes-embedded' : ''}`}
      aria-label="CYAPI 专属线路"
    >
      <div className="n-cyapi-heading">
        <div>
          {!embedded && <h3>CYAPI 专属线路</h3>}
          <p>选择线路，保存后生效。</p>
        </div>
        <div className="n-cyapi-actions">
          <button
            type="button"
            className="n-button"
            disabled={disabled || loading || running}
            onClick={onReload}
          >
            <RefreshCw size={14} aria-hidden="true" className={loading ? 'n-spin' : ''} />
            {loading ? '正在更新' : '更新线路'}
          </button>
          <button
            type="button"
            className="n-button"
            disabled={disabled || loading || running || !nativeAvailable}
            onClick={() => void testRoutes()}
            title={
              !nativeAvailable ? '请在桌面版测延迟' : '不带 API Key 请求 /v1/models，测量响应头延迟'
            }
          >
            {running ? (
              <LoaderCircle size={14} className="n-spin" aria-hidden="true" />
            ) : (
              <Gauge size={14} aria-hidden="true" />
            )}
            {running ? '测量中' : '测延迟'}
          </button>
        </div>
      </div>
      <div
        className="n-cyapi-list"
        role="radiogroup"
        aria-label="选择 CYAPI 线路"
        aria-busy={loading}
      >
        {view.manifest.routes.map((route) => {
          const result = results[route.id];
          const testing = pending.includes(route.id);
          const fastest =
            !running && best !== null && result?.reachable && result.latencyMs === best;
          const active = sameRoute(value, route.baseUrl);
          return (
            <label key={route.id} className={`n-cyapi-route ${active ? 'selected' : ''}`}>
              <input
                type="radio"
                name="cyapi-route"
                value={route.id}
                checked={active}
                disabled={disabled}
                onChange={() => onChange(route.baseUrl)}
              />
              <span className="n-cyapi-route-info">
                <span className="n-cyapi-route-name">
                  {route.name}
                  {route.id === view.manifest.defaultRouteId && (
                    <span className="n-pill">默认</span>
                  )}
                  {fastest && <span className="n-pill n-cyapi-best">延迟最低</span>}
                  {route.baseUrl.startsWith('http:') && (
                    <span className="n-cyapi-http" title="此线路使用未加密的 HTTP 连接">
                      HTTP
                    </span>
                  )}
                </span>
                <code>{route.baseUrl}</code>
              </span>
              <span
                className={`n-cyapi-latency ${result?.reachable ? 'available' : ''}`}
                title={result?.message}
              >
                {testing
                  ? '正在测量…'
                  : result
                    ? result.latencyMs !== null
                      ? `${result.latencyMs} ms${result.reachable ? '' : ` · ${result.message}`}`
                      : result.message
                    : running
                      ? '等待测量'
                      : '未测量'}
              </span>
            </label>
          );
        })}
      </div>
      <p className="n-cyapi-status" role="status" aria-live="polite">
        {loading
          ? view.source === 'default'
            ? '正在获取服务器线路清单…'
            : '已载入本地线路清单，正在后台更新…'
          : running
            ? `已完成 ${completed} / ${view.manifest.routes.length} 条线路`
            : completed
              ? `测量完成，${successful.length} / ${view.manifest.routes.length} 条正常响应（含 401）；每条请求 3 次，取收到响应的平均延迟。`
              : view.message ||
                (view.source === 'server'
                  ? '线路已更新，本地缓存已替换。'
                  : '已载入本地线路缓存。')}
      </p>
      {!selected && (
        <p className="n-config-note">当前地址不在最新线路清单中。选择其他线路前将保留当前地址。</p>
      )}
      <SettingHelp>
        <p className="n-config-note">
          每条线路向模型列表接口（通常为 /v1/models）发送 3 次不带 API Key 的 GET
          请求，计时到收到响应头，不下载模型列表。HTTP 401 属于预期的鉴权响应；其他 HTTP
          状态也显示延迟，但不参与最低延迟推荐。延迟包含连接建立和接口响应，不代表模型生成速度或下载带宽。
          {!nativeAvailable && '请在桌面版测延迟。'}
          线路优先读取本地缓存，启动时自动更新。
        </p>
      </SettingHelp>
    </section>
  );
}
