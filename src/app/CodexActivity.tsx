import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Activity,
  AlertCircle,
  CheckCircle2,
  ChevronRight,
  Clock,
  Download,
  LoaderCircle,
  Pause,
  Play,
  Radio,
  RefreshCw,
  Search,
  ShieldQuestion,
  X,
} from 'lucide-react';
import {
  codexLogDetail,
  codexLogEvents,
  codexLogExport,
  codexLogOverview,
  codexLogSources,
} from '../lib/api/codexLogs';
import type {
  CodexCluster,
  CodexEvent,
  CodexEventKind,
  CodexLogDetail,
  CodexLogSources,
  CodexOverview,
  CodexThreadStat,
} from '../lib/api/codexLogs';
import { nativeAvailable } from '../lib/api/native';
import './diagnostics.css';

const windows = [
  { label: '1 小时', seconds: 3_600 },
  { label: '6 小时', seconds: 21_600 },
  { label: '24 小时', seconds: 86_400 },
  { label: '3 天', seconds: 259_200 },
  { label: '全部', seconds: 0 },
];
const kindTitles: Record<string, string> = {
  request: '请求',
  retry: '重试',
  turn_error: '回合失败',
  sse: '流中断',
  mcp: 'MCP',
  tool: '工具',
  session: '会话',
  error: '告警',
  other: '其他',
};
function time(ms: number) {
  return ms ? new Date(ms).toLocaleTimeString('zh-CN', { hour12: false }) : '—';
}
function dateTime(ms: number) {
  return ms ? new Date(ms).toLocaleString('zh-CN', { hour12: false }) : '—';
}
function size(bytes: number) {
  if (!bytes) return '0 B';
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / 1048576).toFixed(1)} MB`;
}
function threadLabel(thread: CodexThreadStat) {
  return thread.title?.trim() || thread.id.slice(0, 8);
}
function clipLabel(text: string, max: number) {
  return text.length > max ? `${text.slice(0, max)}…` : text;
}
function shortThread(id?: string) {
  return id ? id.slice(0, 8) : '—';
}

function ClusterCard({ cluster, onOpen }: { cluster: CodexCluster; onOpen: (id: number) => void }) {
  return (
    <details
      className={`d-result x-cluster ${cluster.level === 'ERROR' ? 'd-status-fail' : 'd-status-warn'}`}
    >
      <summary>
        <span className="d-result-icon">
          <AlertCircle size={18} />
        </span>
        <span className="d-result-content">
          <strong>{cluster.title || cluster.target}</strong>
          <span>
            {cluster.target} · {cluster.count} 次 · {dateTime(cluster.firstAt)} →{' '}
            {dateTime(cluster.lastAt)}
          </span>
        </span>
        <span className="n-pill">{kindTitles[cluster.level] ?? cluster.level}</span>
        {cluster.benign && <span className="n-pill">低相关</span>}
        <ChevronRight className="d-chevron" size={16} />
      </summary>
      <div className="d-result-body">
        <p className="x-advice">{cluster.advice}</p>
        <code>{cluster.sample}</code>
        {!!cluster.ids.length && (
          <p className="x-links">
            {cluster.ids.map((id) => (
              <button key={id} className="x-link" onClick={() => onOpen(id)}>
                查看记录 #{id}
              </button>
            ))}
          </p>
        )}
      </div>
    </details>
  );
}

export function CodexActivity() {
  const [sources, setSources] = useState<CodexLogSources>();
  const [overview, setOverview] = useState<CodexOverview>();
  const [requests, setRequests] = useState<CodexEvent[]>([]);
  const [windowSeconds, setWindowSeconds] = useState(86_400);
  const [live, setLive] = useState(true);
  const [busy, setBusy] = useState('');
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [tab, setTab] = useState<'overview' | 'requests' | 'threads' | 'errors' | 'stream'>(
    'overview',
  );

  const [requestSearch, setRequestSearch] = useState('');
  const [onlyFailed, setOnlyFailed] = useState(false);

  const [stream, setStream] = useState<CodexEvent[]>([]);
  const [streamKind, setStreamKind] = useState<CodexEventKind | ''>('');
  const [streamLevel, setStreamLevel] = useState('');
  const [streamSearch, setStreamSearch] = useState('');
  const [streamThread, setStreamThread] = useState('');
  const [streamMore, setStreamMore] = useState(false);
  const [streamMissing, setStreamMissing] = useState(false);

  const [detail, setDetail] = useState<CodexLogDetail>();
  const [detailBusy, setDetailBusy] = useState(false);

  const lastId = useRef(0);
  const streamKey = `${streamKind}|${streamLevel}|${streamSearch}|${streamThread}|${windowSeconds}`;

  const refresh = useCallback(
    async (quiet = false) => {
      if (!nativeAvailable) return;
      if (!quiet) setBusy('正在读取 Codex 日志…');
      try {
        const since = windowSeconds ? Date.now() - windowSeconds * 1000 : undefined;
        const [next, page] = await Promise.all([
          codexLogOverview(windowSeconds),
          codexLogEvents({ kinds: ['request'], since, limit: 800 }),
        ]);
        setOverview(next);
        setRequests(page.events);
        setError('');
      } catch (reason) {
        setError(String(reason));
      } finally {
        if (!quiet) setBusy('');
      }
    },
    [windowSeconds],
  );

  const loadStream = useCallback(
    async (filter: { kind?: string; level?: string; search?: string; thread?: string }) => {
      try {
        const page = await codexLogEvents({
          kinds: filter.kind ? [filter.kind as CodexEventKind] : undefined,
          levels: filter.level ? [filter.level] : undefined,
          search: filter.search || undefined,
          threadId: filter.thread || undefined,
          since: windowSeconds ? Date.now() - windowSeconds * 1000 : undefined,
          limit: 300,
        });
        setStream(page.events);
        setStreamMore(page.hasMore);
        setStreamMissing(!!page.missing);
        lastId.current = page.events[0]?.id ?? 0;
      } catch (reason) {
        setError(String(reason));
      }
    },
    [windowSeconds],
  );

  useEffect(() => {
    if (!nativeAvailable) return;
    void codexLogSources()
      .then(setSources)
      .catch((reason) => setError(String(reason)));
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  useEffect(() => {
    if (!live || !nativeAvailable) return;
    const timer = window.setInterval(() => void refresh(true), 4_000);
    return () => window.clearInterval(timer);
  }, [live, refresh]);

  // The stream tab keeps appending new rows; filters reset it.
  useEffect(() => {
    if (tab !== 'stream' || !nativeAvailable) return;
    lastId.current = 0;
    void loadStream({
      kind: streamKind,
      level: streamLevel,
      search: streamSearch,
      thread: streamThread,
    });
  }, [tab, streamKey, loadStream, streamKind, streamLevel, streamSearch, streamThread]);
  useEffect(() => {
    if (tab !== 'stream' || !live || !nativeAvailable) return;
    const timer = window.setInterval(async () => {
      try {
        const page = await codexLogEvents({
          kinds: streamKind ? [streamKind as CodexEventKind] : undefined,
          levels: streamLevel ? [streamLevel] : undefined,
          search: streamSearch || undefined,
          threadId: streamThread || undefined,
          afterId: lastId.current || undefined,
          limit: 200,
        });
        if (!page.events.length) return;
        lastId.current = Math.max(lastId.current, page.events[0].id);
        setStream((current) => [...page.events.reverse(), ...current].slice(0, 1_200));
      } catch {
        // A transient busy database must not interrupt the view.
      }
    }, 2_500);
    return () => window.clearInterval(timer);
  }, [tab, live, streamKind, streamLevel, streamSearch, streamThread]);

  async function openDetail(id: number) {
    setDetailBusy(true);
    try {
      setDetail(await codexLogDetail(id));
    } catch (reason) {
      setError(String(reason));
    } finally {
      setDetailBusy(false);
    }
  }

  const filteredRequests = useMemo(() => {
    const needle = requestSearch.trim().toLowerCase();
    return requests.filter((event) => {
      if (onlyFailed && (event.http?.ok ?? true)) return false;
      if (!needle) return true;
      return `${event.summary} ${event.http?.host ?? ''} ${event.threadId ?? ''} ${event.http?.requestId ?? ''}`
        .toLowerCase()
        .includes(needle);
    });
  }, [requests, requestSearch, onlyFailed]);

  const logSource = overview?.source ?? sources?.logs;
  const missing = !logSource?.readable;

  if (!nativeAvailable) {
    return (
      <div className="d-empty">
        <ShieldQuestion size={28} />
        <h3>请在桌面版使用</h3>
        <p>Codex 日志分析只在本机读取日志数据库，不在网页预览中运行。</p>
      </div>
    );
  }

  return (
    <div className="x-page">
      <section className="n-panel x-source">
        <div className="d-section-title">
          <h3>日志数据库</h3>
          <div className="x-toolbar">
            <select
              aria-label="时间范围"
              value={windowSeconds}
              onChange={(e) => setWindowSeconds(Number(e.target.value))}
            >
              {windows.map((item) => (
                <option key={item.label} value={item.seconds}>
                  最近 {item.label}
                </option>
              ))}
            </select>
            <button
              className={`n-button ${live ? 'n-primary' : ''}`}
              onClick={() => setLive(!live)}
            >
              {live ? <Pause size={14} /> : <Play size={14} />}
              {live ? '停止跟随' : '自动跟随'}
            </button>
            <button className="n-button" disabled={!!busy} onClick={() => void refresh()}>
              <RefreshCw size={14} />
              刷新
            </button>
            <button
              className="n-button"
              disabled={!!busy}
              onClick={() =>
                void (async () => {
                  try {
                    const result = await codexLogExport(windowSeconds);
                    setNotice(`报告已导出：${result.path}`);
                  } catch (reason) {
                    setError(String(reason));
                  }
                })()
              }
            >
              <Download size={14} />
              导出
            </button>
          </div>
        </div>
        {logSource ? (
          <div className="x-source-grid">
            <div>
              <strong>{logSource.name}</strong>
              <span>{logSource.path}</span>
            </div>
            <span>{size(logSource.size)}</span>
            <span>{logSource.rows.toLocaleString('zh-CN')} 条记录</span>
            <span>
              {dateTime(logSource.oldest)} → {dateTime(logSource.newest)}
            </span>
            <span className={logSource.readable ? 'x-ok' : 'd-error'}>
              {logSource.readable ? '只读可用' : (logSource.error ?? '不可读')}
            </span>
          </div>
        ) : (
          <p className="d-help">正在发现日志数据库…</p>
        )}
        {error && (
          <div className="n-alert n-error" role="alert">
            <AlertCircle size={16} />
            <span>{error}</span>
            <button aria-label="关闭" onClick={() => setError('')}>
              <X size={14} />
            </button>
          </div>
        )}
        {notice && (
          <div className="n-alert n-success" role="status">
            <CheckCircle2 size={16} />
            <span>{notice}</span>
            <button aria-label="关闭" onClick={() => setNotice('')}>
              <X size={14} />
            </button>
          </div>
        )}
        {busy && (
          <div className="n-progress" role="status">
            <LoaderCircle size={15} className="n-spin" />
            {busy}
          </div>
        )}
      </section>

      {missing && (
        <section className="n-panel">
          <h3>未找到 Codex 日志数据库</h3>
          <p className="d-help">
            当前 Codex 版本可能未启用 SQLite 日志，或 CODEX_HOME
            指向了其他目录。可在「文本日志」中手动读取其他日志文件； 也可以在 Codex 配置了{' '}
            <code>sqlite_home</code> 后重新检测。
          </p>
        </section>
      )}

      {overview && (
        <>
          <div className="x-totals">
            {[
              ['模型请求', overview.totals.requests],
              ['成功', overview.totals.ok],
              ['失败', overview.totals.failed],
              ['重试', overview.totals.retries],
              ['回合失败', overview.totals.turnErrors],
              ['告警', overview.totals.warnings],
              ['错误', overview.totals.errors],
              ['工具调用', overview.totals.tools],
            ].map(([label, value]) => (
              <div key={label as string} className="x-total">
                <strong>{Number(value).toLocaleString('zh-CN')}</strong>
                <span>{label}</span>
              </div>
            ))}
          </div>

          <nav className="d-tabs" aria-label="日志视图">
            {(
              [
                ['overview', '概览'],
                ['requests', '请求与线路'],
                ['threads', '线程'],
                ['errors', '错误排查'],
                ['stream', '事件流'],
              ] as const
            ).map(([id, title]) => (
              <button
                key={id}
                className={tab === id ? 'active' : ''}
                aria-current={tab === id ? 'page' : undefined}
                onClick={() => setTab(id)}
              >
                {title}
              </button>
            ))}
          </nav>

          {tab === 'overview' && (
            <>
              <section className="n-panel">
                <div className="d-section-title">
                  <h3>线路分布</h3>
                  <span className="n-pill">
                    <Radio size={11} /> {overview.config.expectedHost || '未配置'}
                  </span>
                </div>
                <table className="x-table">
                  <thead>
                    <tr>
                      <th>地址</th>
                      <th>协议</th>
                      <th>请求</th>
                      <th>成功</th>
                      <th>失败</th>
                      <th>状态码</th>
                      <th>线程</th>
                      <th>最近</th>
                    </tr>
                  </thead>
                  <tbody>
                    {overview.routes.map((route) => (
                      <tr key={route.host} className={route.configured ? 'x-row-active' : ''}>
                        <td>
                          {route.host} {route.configured && <span className="n-pill">当前</span>}
                        </td>
                        <td className={route.scheme === 'https' ? 'x-ok' : 'x-warn'}>
                          {route.scheme}
                        </td>
                        <td>{route.requests}</td>
                        <td>{route.ok}</td>
                        <td className={route.failed ? 'd-error' : ''}>{route.failed}</td>
                        <td>
                          {route.statuses.map((item) => (
                            <span key={item.status} className="x-chip">
                              {item.status}×{item.count}
                            </span>
                          ))}
                        </td>
                        <td>{route.threads}</td>
                        <td>{time(route.lastAt)}</td>
                      </tr>
                    ))}
                    {!overview.routes.length && (
                      <tr>
                        <td colSpan={8}>时间窗内没有模型请求。</td>
                      </tr>
                    )}
                  </tbody>
                </table>
              </section>

              <section className="n-panel">
                <div className="d-section-title">
                  <h3>值得关注的事件</h3>
                  <span className="n-pill">{overview.recent.length}</span>
                </div>
                {overview.recent.length ? (
                  <ul className="x-feed">
                    {overview.recent.map((event) => (
                      <li key={event.id} className={event.level === 'ERROR' ? 'd-error' : ''}>
                        <time>{time(event.time)}</time>
                        <span className="n-pill">{kindTitles[event.kind] ?? event.kind}</span>
                        <span className="x-feed-text">{event.summary}</span>
                        <button className="x-link" onClick={() => void openDetail(event.id)}>
                          #{event.id}
                        </button>
                      </li>
                    ))}
                  </ul>
                ) : (
                  <p className="d-help">时间窗内没有重试、回合失败或失败请求。</p>
                )}
              </section>

              <section className="n-panel">
                <h3>说明</h3>
                <ul className="x-notes">
                  {overview.notes.map((note) => (
                    <li key={note}>{note}</li>
                  ))}
                </ul>
              </section>
            </>
          )}

          {tab === 'requests' && (
            <section className="n-panel">
              <div className="d-section-title">
                <h3>模型请求（{filteredRequests.length}）</h3>
                <div className="x-toolbar">
                  <label className="d-search">
                    <Search size={15} />
                    <input
                      aria-label="筛选请求"
                      placeholder="地址、线程、request-id"
                      value={requestSearch}
                      onChange={(e) => setRequestSearch(e.target.value)}
                    />
                  </label>
                  <label className="x-check">
                    <input
                      type="checkbox"
                      checked={onlyFailed}
                      onChange={(e) => setOnlyFailed(e.target.checked)}
                    />
                    只看失败
                  </label>
                </div>
              </div>
              <table className="x-table">
                <thead>
                  <tr>
                    <th>时间</th>
                    <th>地址</th>
                    <th>方法</th>
                    <th>状态</th>
                    <th>线程</th>
                    <th>x-request-id</th>
                    <th className="x-col-action" />
                  </tr>
                </thead>
                <tbody>
                  {filteredRequests.map((event) => (
                    <tr key={event.id}>
                      <td>{time(event.time)}</td>
                      <td>{event.http?.host ?? '—'}</td>
                      <td>{event.http?.method ?? '—'}</td>
                      <td className={event.http?.ok ? 'x-ok' : 'd-error'}>
                        {event.http?.ok
                          ? `${event.http?.status ?? ''} ${event.http?.reason ?? ''}`.trim()
                          : event.http?.timeout
                            ? '超时'
                            : event.http?.connect
                              ? '连接失败'
                              : '失败'}
                      </td>
                      <td title={event.threadId}>{shortThread(event.threadId)}</td>
                      <td className="x-mono">{event.http?.requestId ?? '—'}</td>
                      <td className="x-col-action">
                        <button className="x-link" onClick={() => void openDetail(event.id)}>
                          详情
                        </button>
                      </td>
                    </tr>
                  ))}
                  {!filteredRequests.length && (
                    <tr>
                      <td colSpan={7}>没有匹配的请求。</td>
                    </tr>
                  )}
                </tbody>
              </table>
            </section>
          )}

          {tab === 'threads' && (
            <section className="n-panel">
              <div className="d-section-title">
                <h3>线程与线路归属</h3>
                <span className="n-pill">{overview.threads.length}</span>
              </div>
              <table className="x-table x-threads">
                <thead>
                  <tr>
                    <th>会话</th>
                    <th>provider</th>
                    <th>模型</th>
                    <th>请求</th>
                    <th>失败</th>
                    <th>实际地址</th>
                    <th>归属</th>
                    <th>最近</th>
                  </tr>
                </thead>
                <tbody>
                  {overview.threads.map((thread) => (
                    <tr key={thread.id}>
                      <td title={`${threadLabel(thread)} · ${thread.id}`}>
                        <span className="x-thread-title">{threadLabel(thread)}</span>
                      </td>
                      <td title={thread.provider ?? undefined}>{thread.provider ?? '未知'}</td>
                      <td title={thread.model ?? undefined}>{thread.model ?? '—'}</td>
                      <td>{thread.requests}</td>
                      <td className={thread.failures ? 'd-error' : ''}>{thread.failures}</td>
                      <td className="x-mono">
                        {thread.hosts.length
                          ? thread.hosts.map((host) => (
                              <span key={host} className="x-host">
                                {host}
                              </span>
                            ))
                          : '—'}
                      </td>
                      <td className={thread.routeOk ? 'x-ok' : 'x-warn'}>
                        {thread.routeOk ? '与配置一致' : thread.provider ? '其他线路' : '无法核对'}
                      </td>
                      <td>{time(thread.lastAt)}</td>
                    </tr>
                  ))}
                  {!overview.threads.length && (
                    <tr>
                      <td colSpan={8}>时间窗内没有线程活动。</td>
                    </tr>
                  )}
                </tbody>
              </table>
            </section>
          )}

          {tab === 'errors' && (
            <section className="d-log-analysis">
              {overview.errors.map((cluster) => (
                <ClusterCard
                  key={`${cluster.target}-${cluster.signature}`}
                  cluster={cluster}
                  onOpen={(id) => void openDetail(id)}
                />
              ))}
              {!overview.errors.length && (
                <div className="d-empty">
                  <CheckCircle2 size={28} />
                  <h3>时间窗内没有告警或错误</h3>
                  <p>缩小时间范围或切换到「事件流」查看全部记录。</p>
                </div>
              )}
            </section>
          )}

          {tab === 'stream' && (
            <section className="n-panel">
              <div className="d-section-title">
                <h3>事件流（最近 {stream.length} 条）</h3>
                <div className="x-toolbar">
                  <select
                    value={streamKind}
                    onChange={(e) => setStreamKind(e.target.value as CodexEventKind | '')}
                  >
                    <option value="">全部类型</option>
                    {Object.entries(kindTitles).map(([id, title]) => (
                      <option key={id} value={id}>
                        {title}
                      </option>
                    ))}
                  </select>
                  <select value={streamLevel} onChange={(e) => setStreamLevel(e.target.value)}>
                    <option value="">全部级别</option>
                    {['TRACE', 'DEBUG', 'INFO', 'WARN', 'ERROR'].map((level) => (
                      <option key={level} value={level}>
                        {level}
                      </option>
                    ))}
                  </select>
                  <label className="d-search">
                    <Search size={15} />
                    <input
                      aria-label="搜索日志内容"
                      placeholder="搜索内容或模块"
                      value={streamSearch}
                      onChange={(e) => setStreamSearch(e.target.value)}
                    />
                  </label>
                  <select value={streamThread} onChange={(e) => setStreamThread(e.target.value)}>
                    <option value="">全部线程</option>
                    {overview.threads.slice(0, 60).map((thread) => (
                      <option key={thread.id} value={thread.id}>
                        {clipLabel(threadLabel(thread), 32)}
                      </option>
                    ))}
                  </select>
                  <button
                    className="n-button"
                    onClick={() =>
                      void loadStream({
                        kind: streamKind,
                        level: streamLevel,
                        search: streamSearch,
                        thread: streamThread,
                      })
                    }
                  >
                    <RefreshCw size={14} />
                    重新加载
                  </button>
                </div>
              </div>
              {streamMissing && <p className="d-help">未找到日志数据库，事件流不可用。</p>}
              <ul className="x-stream">
                {stream.map((event) => (
                  <li key={event.id} className={`x-level-${event.level.toLowerCase()}`}>
                    <button onClick={() => void openDetail(event.id)}>
                      <time>{time(event.time)}</time>
                      <span className="x-mono x-stream-level">{event.level}</span>
                      <span className="x-stream-kind">{kindTitles[event.kind] ?? event.kind}</span>
                      <span className="x-stream-summary">{event.summary}</span>
                      <span className="x-mono x-stream-target">{event.target}</span>
                    </button>
                  </li>
                ))}
                {!stream.length && <li className="d-help">没有匹配的记录。</li>}
              </ul>
              {streamMore && (
                <p className="d-help">仅显示最近 300 条；可用搜索或时间范围缩小结果。</p>
              )}
            </section>
          )}

          {detailBusy && !detail && (
            <div className="n-progress" role="status">
              <LoaderCircle size={15} className="n-spin" />
              正在读取记录…
            </div>
          )}
          {detail && (
            <section className="n-panel x-detail">
              <div className="d-section-title">
                <h3>
                  <Clock size={14} /> #{detail.id} · {dateTime(detail.time)}
                </h3>
                <button className="n-button" onClick={() => setDetail(undefined)}>
                  <X size={14} />
                  关闭
                </button>
              </div>
              <div className="x-detail-meta">
                <span className="n-pill">{detail.level}</span>
                <code>{detail.target}</code>
                {detail.threadId && <code>thread {detail.threadId.slice(0, 8)}</code>}
                {detail.pid != null && <code>pid {detail.pid}</code>}
              </div>
              {detail.span && (
                <details className="x-detail-span">
                  <summary>跨度上下文</summary>
                  <pre>{detail.span}</pre>
                </details>
              )}
              <pre>{detail.message}</pre>
            </section>
          )}
        </>
      )}

      {!overview && busy && (
        <div className="n-progress" role="status">
          <LoaderCircle size={15} className="n-spin" />
          {busy}
        </div>
      )}
      {!overview && !busy && (
        <div className="d-empty">
          <Activity size={32} />
          <h3>正在后台读取 Codex 日志</h3>
          <p>日志只在本地只读解析，不会上传。</p>
        </div>
      )}
    </div>
  );
}
