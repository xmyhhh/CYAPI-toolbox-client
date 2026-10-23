import { useEffect, useRef, useState } from 'react';
import { Gauge, LoaderCircle, RefreshCw } from 'lucide-react';
import { nativeAvailable } from '../lib/api/native';
import { probeCyapiRoute, sameRoute } from '../lib/api/cyapi';
import type { CyapiMeasurement, CyapiRoutesView } from '../lib/api/cyapi';

interface Props {
  view: CyapiRoutesView;
  loading: boolean;
  disabled: boolean;
  value: string;
  onChange: (value: string) => void;
  onReload: () => void;
}
export function CyapiRoutes({ view, loading, disabled, value, onChange, onReload }: Props) {
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
    <section className="n-cyapi-routes" aria-label="CYAPI 专属线路">
      <div className="n-cyapi-heading">
        <div>
          <h3>CYAPI 专属线路</h3>
          <p>默认使用官方域名。测速后由你选择，保存配置后生效。</p>
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
            title={!nativeAvailable ? '请在桌面版测速' : undefined}
          >
            {running ? (
              <LoaderCircle size={14} className="n-spin" aria-hidden="true" />
            ) : (
              <Gauge size={14} aria-hidden="true" />
            )}
            {running ? '测速中' : '线路测速'}
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
                disabled={disabled || loading}
                onChange={() => onChange(route.baseUrl)}
              />
              <span className="n-cyapi-route-info">
                <span className="n-cyapi-route-name">
                  {route.name}
                  {route.id === view.manifest.defaultRouteId && (
                    <span className="n-pill">默认</span>
                  )}
                  {fastest && <span className="n-pill n-cyapi-best">连接最快</span>}
                  {route.baseUrl.startsWith('http:') && (
                    <span className="n-cyapi-http" title="此线路使用未加密的 HTTP 连接">
                      HTTP
                    </span>
                  )}
                </span>
                <code>{route.baseUrl}</code>
              </span>
              <span className={`n-cyapi-latency ${result?.reachable ? 'available' : ''}`}>
                {testing
                  ? '正在测速…'
                  : result
                    ? result.reachable
                      ? `${result.latencyMs} ms`
                      : result.message
                    : running
                      ? '等待测速'
                      : '未测速'}
              </span>
            </label>
          );
        })}
      </div>
      <p className="n-cyapi-status" role="status" aria-live="polite">
        {loading
          ? '正在获取服务器线路清单…'
          : running
            ? `已完成 ${completed} / ${view.manifest.routes.length} 条线路`
            : completed
              ? `测速完成，${successful.length} / ${view.manifest.routes.length} 条可连接（每条 3 次取平均）。请选择要使用的线路。`
              : view.message || '备用线路由 CYAPI 服务器维护。'}
      </p>
      {!selected && (
        <p className="n-config-note">当前地址不在最新线路清单中。选择其他线路前将保留当前地址。</p>
      )}
      <p className="n-config-note">
        测速不发送 API Key，不调用模型；每条线路发送 3 次不带密钥的请求并取平均延迟。
        {!nativeAvailable && '请在桌面版测速。'}
      </p>
    </section>
  );
}
