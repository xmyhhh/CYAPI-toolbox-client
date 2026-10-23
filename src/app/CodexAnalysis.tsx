import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  Activity,
  AlertCircle,
  ArrowRight,
  CheckCircle2,
  ChevronRight,
  Database,
  Download,
  FileText,
  FolderOpen,
  LoaderCircle,
  Pause,
  Play,
  RefreshCw,
  Search,
} from 'lucide-react';
import { Modal } from '../components/Primitives';
import { nativeAvailable } from '../lib/api/native';
import {
  analyzeCodex,
  analysisEvents,
  analysisDetail,
  analyzeSession,
  exportAnalysis,
} from '../lib/api/codexAnalysis';
import type {
  Analysis,
  AnalysisDetail,
  AnalysisEvent,
  AnalysisFilter,
  AnalysisIssue,
  AnalysisPage,
  AnalysisSession,
  SessionAnalysis,
} from '../lib/api/codexAnalysis';
import './diagnostics.css';
import './codex-analysis.css';

const windows = [
  ['1 小时', 3600],
  ['6 小时', 21600],
  ['24 小时', 86400],
  ['3 天', 259200],
  ['7 天', 604800],
  ['全部', 0],
] as const;
const roles: Record<string, string> = {
  runtime: '运行数据库',
  index: '会话索引',
  cli: 'CLI 日志',
  desktop: '桌面日志',
  rollout: '会话记录',
};
const kinds: Record<string, string> = {
  request: '模型请求',
  http: '其他 HTTP',
  retry: '重试',
  turn_error: '回合错误',
  sse: '响应流',
  mcp: 'MCP',
  tool: '工具调用',
  tool_result: '工具返回',
  session: '会话初始化',
  error: '运行告警',
  desktop: '桌面事件',
  other: '其他',
  turn_started: '回合开始',
  turn_completed: '回合完成',
  turn_aborted: '回合中止',
  context: '回合配置',
  compaction: '上下文压缩',
};
const categories: Record<string, string> = {
  auth: '认证',
  rate_limit: '限流',
  upstream: '上游服务',
  network: '网络',
  stream: '响应流',
  mcp: 'MCP',
  tool: '工具与权限',
  retry: '重试',
  turn: '回合',
  desktop: '桌面',
  background: '后台功能',
  runtime: '运行时',
};
const date = (ms: number) => (ms ? new Date(ms).toLocaleString('zh-CN', { hour12: false }) : '—');
const clock = (ms: number) =>
  ms ? new Date(ms).toLocaleTimeString('zh-CN', { hour12: false }) : '—';
const number = (n: number) => n.toLocaleString('zh-CN');
const bytes = (n: number) =>
  n >= 1048576 ? `${(n / 1048576).toFixed(1)} MB` : `${(n / 1024).toFixed(1)} KB`;
const title = (s: AnalysisSession) => s.title.trim() || s.id.slice(0, 8);
const reason = (e: unknown) => (e instanceof Error ? e.message : String(e));
type Tab = 'overview' | 'sessions' | 'requests' | 'issues' | 'events' | 'sources';

export interface CodexAnalysisClient {
  available: boolean;
  analyze: typeof analyzeCodex;
  events: typeof analysisEvents;
  detail: typeof analysisDetail;
  session: typeof analyzeSession;
  export: typeof exportAnalysis;
}
const localClient: CodexAnalysisClient = {
  available: nativeAvailable,
  analyze: analyzeCodex,
  events: analysisEvents,
  detail: analysisDetail,
  session: analyzeSession,
  export: exportAnalysis,
};

function EventList({
  events,
  requests = false,
  onOpen,
  onSession,
}: {
  events: AnalysisEvent[];
  requests?: boolean;
  onOpen: (event: AnalysisEvent) => void;
  onSession: (id: string) => void;
}) {
  if (!events.length) return <div className="ca-empty">当前读取范围内没有匹配的记录。</div>;
  return (
    <div className="ca-table-scroll">
      <table className="x-table ca-events">
        <thead>
          <tr>
            <th>时间 / 来源</th>
            <th>{requests ? '请求地址 / 结果' : '事件'}</th>
            <th>会话</th>
            <th />
          </tr>
        </thead>
        <tbody>
          {events.map((e) => (
            <tr key={e.id}>
              <td>
                <time title={date(e.time)}>{clock(e.time)}</time>
                <small>
                  {roles[e.location.sourceRole] ?? e.location.sourceRole}
                  {e.mirrors.length > 0 && ' · 已合并镜像'}
                </small>
              </td>
              <td>
                <div className="ca-event-title">
                  <span
                    className={`ca-tag ${(e.http && !e.http.ok) || e.level === 'ERROR' ? 'ca-tag-error' : e.level === 'WARN' ? 'ca-tag-warn' : ''}`}
                  >
                    {e.http
                      ? (e.http.status ?? (e.http.timeout ? '超时' : '连接失败'))
                      : (kinds[e.kind] ?? e.kind)}
                  </span>
                  <span>
                    {requests && e.http
                      ? `${e.http.method} ${e.http.host}${e.http.path}`
                      : e.summary}
                  </span>
                </div>
                <small className="ca-mono">
                  {requests
                    ? `request-id: ${e.http?.requestId ?? '未记录'}`
                    : `${e.level} · ${e.target}`}
                </small>
              </td>
              <td>
                {e.threadId ? (
                  <button
                    className="x-link ca-mono"
                    onClick={() => onSession(e.threadId!)}
                    title={e.threadId}
                  >
                    {e.threadId.slice(0, 8)}
                  </button>
                ) : (
                  <span className="ca-muted">未关联</span>
                )}
              </td>
              <td>
                <button className="x-link" onClick={() => onOpen(e)}>
                  证据
                </button>
              </td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}

function IssueCard({
  issue,
  onOpen,
  onSession,
}: {
  issue: AnalysisIssue;
  onOpen: (id: string) => void;
  onSession: (id: string) => void;
}) {
  return (
    <details className={`ca-issue ca-issue-${issue.severity}`}>
      <summary>
        <AlertCircle size={17} />
        <span>
          <strong>{issue.title}</strong>
          <small>
            {categories[issue.category]} · {issue.count} 条记录 · 最近 {date(issue.lastAt)}
          </small>
        </span>
        <ChevronRight size={16} className="d-chevron" />
      </summary>
      <div className="ca-issue-body">
        <p>{issue.advice}</p>
        <pre>{issue.sample}</pre>
        <small>
          首次 {date(issue.firstAt)} · 最近 {date(issue.lastAt)}
        </small>
        <div className="ca-actions">
          {issue.eventIds.map((id, i) => (
            <button key={id} className="x-link" onClick={() => onOpen(id)}>
              证据 {i + 1}
            </button>
          ))}
          {issue.threads.slice(0, 3).map((id) => (
            <button key={id} className="x-link" onClick={() => onSession(id)}>
              会话 {id.slice(0, 8)}
            </button>
          ))}
        </div>
      </div>
    </details>
  );
}

export function CodexAnalysis({ client = localClient }: { client?: CodexAnalysisClient } = {}) {
  const {
    available: nativeAvailable,
    analyze: analyzeCodex,
    events: analysisEvents,
    detail: analysisDetail,
    session: analyzeSession,
    export: exportAnalysis,
  } = client;
  const [analysis, setAnalysis] = useState<Analysis>();
  const [windowSeconds, setWindowSeconds] = useState(86400);
  const [tab, setTab] = useState<Tab>('overview');
  const [live, setLive] = useState(false);
  const [busy, setBusy] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [page, setPage] = useState<AnalysisPage>();
  const [eventsBusy, setEventsBusy] = useState(false);
  const [search, setSearch] = useState('');
  const [debouncedSearch, setDebouncedSearch] = useState('');
  const [sourceRole, setSourceRole] = useState('');
  const [eventKind, setEventKind] = useState('');
  const [level, setLevel] = useState('');
  const [thread, setThread] = useState('');
  const [onlyFailed, setOnlyFailed] = useState(false);
  const [sessionSearch, setSessionSearch] = useState('');
  const [selectedSession, setSelectedSession] = useState('');
  const [session, setSession] = useState<SessionAnalysis>();
  const [sessionBusy, setSessionBusy] = useState(false);
  const [sessionError, setSessionError] = useState('');
  const [issueCategory, setIssueCategory] = useState('');
  const [detail, setDetail] = useState<AnalysisDetail>();
  const [detailBusy, setDetailBusy] = useState(false);
  const refreshTicket = useRef(0);
  const refreshPending = useRef(false);
  const eventsTicket = useRef(0);
  const detailTicket = useRef(0);

  const refresh = useCallback(async () => {
    if (!nativeAvailable) return;
    const ticket = ++refreshTicket.current;
    refreshPending.current = true;
    setBusy(true);
    setError('');
    try {
      const next = await analyzeCodex(windowSeconds);
      if (ticket === refreshTicket.current) setAnalysis(next);
    } catch (e) {
      if (ticket === refreshTicket.current) setError(reason(e));
    } finally {
      if (ticket === refreshTicket.current) {
        setBusy(false);
        refreshPending.current = false;
      }
    }
  }, [windowSeconds, nativeAvailable, analyzeCodex]);
  useEffect(() => {
    setAnalysis(undefined);
    setPage(undefined);
    setSession(undefined);
    setNotice('');
    void refresh();
    return () => {
      refreshTicket.current++;
    };
  }, [refresh]);
  useEffect(() => {
    if (!live || !nativeAvailable) return;
    const timer = window.setInterval(() => {
      if (!refreshPending.current) void refresh();
    }, 15000);
    return () => window.clearInterval(timer);
  }, [live, refresh, nativeAvailable]);
  useEffect(() => {
    const timer = window.setTimeout(() => setDebouncedSearch(search), 200);
    return () => window.clearTimeout(timer);
  }, [search]);
  const filter = useMemo<AnalysisFilter>(
    () => ({
      kinds: tab === 'requests' ? ['request'] : eventKind ? [eventKind] : undefined,
      levels: tab === 'events' && level ? [level] : undefined,
      threadId: thread || undefined,
      search: debouncedSearch || undefined,
      outcome: tab === 'requests' && onlyFailed ? 'failed' : undefined,
    }),
    [tab, eventKind, level, thread, debouncedSearch, onlyFailed],
  );
  useEffect(() => {
    const ticket = ++eventsTicket.current;
    if (!analysis || (tab !== 'events' && tab !== 'requests')) return;
    setEventsBusy(true);
    setPage(undefined);
    void analysisEvents(analysis.id, filter, sourceRole || undefined)
      .then((next) => {
        if (ticket === eventsTicket.current) setPage(next);
      })
      .catch((e) => {
        if (ticket === eventsTicket.current) setError(reason(e));
      })
      .finally(() => {
        if (ticket === eventsTicket.current) setEventsBusy(false);
      });
    return () => {
      eventsTicket.current++;
    };
  }, [analysis, tab, filter, sourceRole, analysisEvents]);
  async function loadMore() {
    if (!analysis || eventsBusy || page?.nextOffset == null) return;
    const ticket = ++eventsTicket.current;
    setEventsBusy(true);
    try {
      const next = await analysisEvents(
        analysis.id,
        filter,
        sourceRole || undefined,
        page.nextOffset,
      );
      if (ticket === eventsTicket.current)
        setPage((prev) => ({ ...next, events: [...(prev?.events ?? []), ...next.events] }));
    } catch (e) {
      if (ticket === eventsTicket.current) setError(reason(e));
    } finally {
      if (ticket === eventsTicket.current) setEventsBusy(false);
    }
  }
  const sessions = useMemo(() => {
    const needle = sessionSearch.trim().toLowerCase();
    return (
      analysis?.sessions.filter(
        (s) =>
          !needle ||
          `${s.title} ${s.id} ${s.cwd} ${s.provider} ${s.model}`.toLowerCase().includes(needle),
      ) ?? []
    );
  }, [analysis, sessionSearch]);
  const effectiveSession = sessions.find((s) => s.id === selectedSession) ?? sessions[0];
  useEffect(() => {
    if (!analysis || tab !== 'sessions' || !effectiveSession) {
      setSession(undefined);
      setSessionBusy(false);
      setSessionError('');
      return;
    }
    let current = true;
    setSessionBusy(true);
    setSession(undefined);
    setSessionError('');
    void analyzeSession(analysis.id, effectiveSession.id)
      .then((next) => {
        if (current) setSession(next);
      })
      .catch((e) => {
        if (current) setSessionError(reason(e));
      })
      .finally(() => {
        if (current) setSessionBusy(false);
      });
    return () => {
      current = false;
    };
  }, [analysis, tab, effectiveSession?.id, analyzeSession]);
  function openSession(id: string) {
    setSessionSearch('');
    setSelectedSession(id);
    setTab('sessions');
  }
  async function openEvidence(eventOrId: AnalysisEvent | string) {
    if (!analysis) return;
    const ticket = ++detailTicket.current;
    setDetailBusy(true);
    setDetail(undefined);
    try {
      const event = typeof eventOrId === 'string' ? undefined : eventOrId;
      const next =
        event?.location.sourceRole === 'rollout'
          ? {
              event,
              message: session?.eventDetails[event.id] ?? event.summary,
              span: '',
              note: '会话 JSONL 的结构信息；不含聊天正文、工具参数和输出。',
            }
          : await analysisDetail(
              analysis.id,
              typeof eventOrId === 'string' ? eventOrId : eventOrId.id,
            );
      if (ticket === detailTicket.current) setDetail(next);
    } catch (e) {
      if (ticket === detailTicket.current) setError(reason(e));
    } finally {
      if (ticket === detailTicket.current) setDetailBusy(false);
    }
  }
  async function saveReport() {
    if (!analysis || exporting) return;
    setExporting(true);
    setNotice('');
    try {
      const result = await exportAnalysis(
        analysis.id,
        tab === 'sessions' ? effectiveSession?.id : undefined,
      );
      setNotice(`报告已保存：${result.path}`);
    } catch (e) {
      setError(reason(e));
    } finally {
      setExporting(false);
    }
  }
  const readableSources =
    analysis?.sources.filter((s) => s.status === 'available' && s.role !== 'rollout').length ?? 0;
  const issues =
    analysis?.issues.filter((i) => !issueCategory || i.category === issueCategory) ?? [];
  const modelSourceAvailable = analysis?.sources.some(
    (s) => ['runtime', 'cli'].includes(s.role) && s.status === 'available',
  );

  return (
    <div className="ca-page">
      <header className="ca-header">
        <div>
          <span className="ca-eyebrow">
            <Activity size={14} /> CODEX
          </span>
          <h2>日志分析</h2>
          <p>关联模型请求、桌面运行与会话回合，沿记录定位问题。</p>
        </div>
        <div className="ca-actions">
          <select
            aria-label="分析时间范围"
            value={windowSeconds}
            disabled={busy}
            onChange={(e) => setWindowSeconds(Number(e.target.value))}
          >
            {windows.map(([label, seconds]) => (
              <option value={seconds} key={seconds}>
                {label}
              </option>
            ))}
          </select>
          <button
            className={`n-button ${live ? 'ca-live' : ''}`}
            disabled={!nativeAvailable}
            onClick={() => setLive(!live)}
            title="每 15 秒更新分析快照"
          >
            {live ? <Pause size={14} /> : <Play size={14} />}
            {live ? '暂停更新' : '自动更新'}
          </button>
          <button
            className="n-button"
            disabled={busy || !nativeAvailable}
            onClick={() => void refresh()}
          >
            <RefreshCw size={14} className={busy ? 'n-spin' : ''} />
            刷新
          </button>
          <button
            className="n-button"
            disabled={!analysis || exporting}
            onClick={() => void saveReport()}
          >
            <Download size={14} />
            {exporting ? '导出中…' : tab === 'sessions' ? '导出会话报告' : '导出报告'}
          </button>
        </div>
      </header>
      {!nativeAvailable && (
        <div className="n-alert">
          <FileText size={16} />
          <span>请在桌面版使用日志分析，直接读取本机 Codex 数据。</span>
        </div>
      )}
      {error && (
        <div className="n-alert n-error" role="alert">
          <AlertCircle size={16} />
          <span>{error}</span>
        </div>
      )}
      {notice && (
        <div className="n-alert ca-notice" role="status">
          <CheckCircle2 size={16} />
          <span>{notice}</span>
        </div>
      )}
      {busy && (
        <div className="n-progress" role="status">
          <LoaderCircle size={15} className="n-spin" />
          正在关联 Codex 日志与会话…
        </div>
      )}
      {analysis && (
        <>
          <div className={`ca-coverage ${analysis.partial ? 'ca-coverage-partial' : ''}`}>
            {analysis.partial ? <AlertCircle size={16} /> : <Database size={16} />}
            <span>
              <strong>
                {analysis.partial
                  ? '统计基于部分记录'
                  : `已读取 ${number(analysis.eventCount)} 条事件`}
              </strong>
              <small>
                {readableSources} 个运行 / 索引来源 · {number(analysis.totals.sessions)} 个关联会话
                · 更新于 {clock(analysis.generatedAt)}
              </small>
            </span>
            <button className="x-link" onClick={() => setTab('sources')}>
              查看读取范围 <ArrowRight size={12} />
            </button>
          </div>
          <nav className="d-tabs ca-tabs" aria-label="Codex 日志分析">
            {(
              [
                ['overview', '概览'],
                ['sessions', '会话分析'],
                ['requests', '模型请求'],
                ['issues', '故障归类'],
                ['events', '全部事件'],
                ['sources', '数据来源'],
              ] as const
            ).map(([id, label]) => (
              <button
                key={id}
                className={tab === id ? 'active' : ''}
                aria-current={tab === id ? 'page' : undefined}
                onClick={() => setTab(id)}
              >
                {label}
                {id === 'issues' && analysis.issues.length > 0 && (
                  <span className="ca-count">{analysis.issues.length}</span>
                )}
              </button>
            ))}
          </nav>
          {tab === 'overview' && (
            <>
              <div className="ca-metrics">
                {(
                  [
                    ['模型请求', analysis.totals.requests, '实际记录的 HTTP 尝试', 'requests'],
                    [
                      'HTTP 失败',
                      analysis.totals.failed,
                      `${analysis.totals.ok} 次 HTTP 成功`,
                      'requests',
                    ],
                    ['请求重试', analysis.totals.retries, '与请求次数分别统计', 'issues'],
                    ['回合错误', analysis.totals.turnErrors, '不由 HTTP 状态推测', 'issues'],
                    ['关联会话', analysis.totals.sessions, '含无请求日志的会话', 'sessions'],
                  ] as const
                ).map(([label, value, help, target]) => (
                  <button
                    key={label}
                    className={`ca-metric ${label === 'HTTP 失败' && value ? 'ca-metric-error' : ''}`}
                    onClick={() => {
                      setTab(target);
                      setOnlyFailed(label === 'HTTP 失败');
                    }}
                  >
                    <span>{label}</span>
                    <strong>{number(value)}</strong>
                    <small>{help}</small>
                  </button>
                ))}
              </div>
              {!modelSourceAvailable && (
                <div className="n-alert">
                  <AlertCircle size={16} />
                  <span>未读到模型运行日志。请求数为当前可见记录，不代表 Codex 没有发起请求。</span>
                </div>
              )}
              <div className="ca-overview-grid">
                <section className="n-panel">
                  <div className="d-section-title">
                    <h3>优先排查</h3>
                    <button className="x-link" onClick={() => setTab('issues')}>
                      全部归类 <ArrowRight size={12} />
                    </button>
                  </div>
                  {analysis.issues.slice(0, 4).map((issue) => (
                    <IssueCard
                      key={issue.id}
                      issue={issue}
                      onOpen={(id) => void openEvidence(id)}
                      onSession={openSession}
                    />
                  ))}
                  {!analysis.issues.length && (
                    <div className="ca-empty">
                      <CheckCircle2 size={22} />
                      <p>已读取记录中未发现匹配的告警。</p>
                      <small>可打开会话查看回合结束情况。</small>
                    </div>
                  )}
                </section>
                <section className="n-panel">
                  <div className="d-section-title">
                    <h3>实际模型请求地址</h3>
                    <button className="x-link" onClick={() => setTab('requests')}>
                      请求明细 <ArrowRight size={12} />
                    </button>
                  </div>
                  <div className="ca-config">
                    <small>当前全局配置</small>
                    <strong>
                      {analysis.config.provider || '未读取'} ·{' '}
                      {analysis.config.model || '未指定模型'}
                    </strong>
                    <code>{analysis.config.baseUrl || '未读取到 base_url'}</code>
                  </div>
                  {analysis.routes.map((r) => (
                    <div className="ca-route" key={r.address}>
                      <code>{r.address}</code>
                      <div>
                        <span>{r.requests} 次尝试</span>
                        <span className="x-ok">{r.ok} 成功</span>
                        <span className={r.failed ? 'd-error' : ''}>{r.failed} 失败</span>
                      </div>
                      <small>
                        {Object.entries(r.statuses)
                          .map(([status, count]) => `${status} × ${count}`)
                          .join(' · ')}
                      </small>
                    </div>
                  ))}
                  {!analysis.routes.length && (
                    <div className="ca-empty">所选范围没有可识别的模型 HTTP 记录。</div>
                  )}
                  <p className="d-help">
                    HTTP 成功表示收到正常响应；回合是否完成，请查看会话时间线。地址不同需结合会话
                    provider 和配置覆盖判断。
                  </p>
                </section>
              </div>
              <section className="n-panel">
                <div className="d-section-title">
                  <h3>最近告警与重试</h3>
                </div>
                <EventList
                  events={analysis.recent}
                  onOpen={(e) => void openEvidence(e)}
                  onSession={openSession}
                />
              </section>
            </>
          )}
          {tab === 'sessions' && (
            <section className="ca-session-layout">
              <aside className="n-panel ca-session-list">
                <label className="ca-search">
                  <Search size={15} />
                  <input
                    aria-label="搜索会话"
                    placeholder="会话、项目、模型"
                    value={sessionSearch}
                    onChange={(e) => setSessionSearch(e.target.value)}
                  />
                </label>
                <small className="ca-list-count">{sessions.length} 个会话 · 最近活动排序</small>
                <div className="ca-session-items">
                  {sessions.map((s) => (
                    <button
                      key={s.id}
                      className={`ca-session-item ${effectiveSession?.id === s.id ? 'active' : ''}`}
                      onClick={() => setSelectedSession(s.id)}
                    >
                      <strong>{title(s)}</strong>
                      <small title={s.cwd}>{s.cwd || '项目路径未记录'}</small>
                      <span>
                        <small>{s.model || s.provider || s.id.slice(0, 8)}</small>
                        <time>{clock(s.updatedAt)}</time>
                      </span>
                      {(s.failures > 0 || s.turnErrors > 0 || s.archived) && (
                        <span>
                          {s.failures > 0 && <em>{s.failures} 次请求失败</em>}
                          {s.turnErrors > 0 && <em>{s.turnErrors} 条回合错误</em>}
                          {s.archived && <small>已归档</small>}
                        </span>
                      )}
                    </button>
                  ))}
                </div>
                {!sessions.length && (
                  <div className="ca-empty">没有匹配会话。可扩大时间范围，或查看数据来源。</div>
                )}
              </aside>
              <div className="ca-session-content">
                {sessionBusy && (
                  <div className="n-progress" role="status">
                    <LoaderCircle size={15} className="n-spin" />
                    正在解析会话回合与用量…
                  </div>
                )}
                {sessionError && (
                  <div className="n-alert n-error" role="alert">
                    {sessionError}
                  </div>
                )}
                {session && (
                  <>
                    <section className="n-panel">
                      <div className="ca-session-heading">
                        <h3>{title(session.session)}</h3>
                        <span className="ca-tag">
                          {session.session.archived
                            ? '已归档'
                            : session.session.origin || '本地会话'}
                        </span>
                      </div>
                      <p className="ca-mono ca-muted">{session.session.id}</p>
                      <div className="ca-session-meta">
                        <span>
                          <FolderOpen size={14} />
                          {session.session.cwd || '未记录项目路径'}
                        </span>
                        <span>provider：{session.session.provider || '未记录'}</span>
                        <span>模型：{session.session.model || '未记录'}</span>
                      </div>
                      <div className="ca-session-stats">
                        <span>
                          <strong>{session.turns.started}</strong> 回合开始
                        </span>
                        <span>
                          <strong>{session.turns.completed}</strong> 完成
                        </span>
                        <span>
                          <strong>{session.turns.aborted}</strong> 中止
                        </span>
                        <span>
                          <strong>{session.toolCalls}</strong> 工具调用
                        </span>
                        <span>
                          <strong>{session.session.requests}</strong> HTTP 尝试
                        </span>
                      </div>
                      {session.session.hosts.length > 0 && (
                        <p className="d-help">
                          请求地址：{session.session.hosts.join('、')}
                          {session.session.routeStatus === 'different' &&
                            ' · 与当前全局配置不同，需核对项目 / profile 或旧会话配置'}
                        </p>
                      )}
                      {session.source.status !== 'available' && (
                        <div className="n-alert">
                          <AlertCircle size={15} />
                          <span>会话文件不可读，仍显示已关联运行日志。{session.source.note}</span>
                        </div>
                      )}
                      {session.source.partial && (
                        <div className="n-alert">
                          <AlertCircle size={15} />
                          <span>{session.source.note}</span>
                        </div>
                      )}
                      <details className="ca-read-scope">
                        <summary>会话文件与统计范围</summary>
                        <code>{session.source.path}</code>
                        <p>{session.note}</p>
                      </details>
                    </section>
                    <section className="n-panel">
                      <div className="d-section-title">
                        <h3>会话累计用量</h3>
                        {session.usage && (
                          <small className="ca-muted">
                            记录于 {date(session.usage.observedAt)}
                          </small>
                        )}
                      </div>
                      {session.usage ? (
                        <>
                          <div className="ca-usage">
                            <div>
                              <small>总 token</small>
                              <strong>{number(session.usage.total)}</strong>
                            </div>
                            <div>
                              <small>输入</small>
                              <strong>{number(session.usage.input)}</strong>
                            </div>
                            <div>
                              <small>其中缓存输入</small>
                              <strong>{number(session.usage.cachedInput)}</strong>
                            </div>
                            <div>
                              <small>输出</small>
                              <strong>{number(session.usage.output)}</strong>
                            </div>
                            <div>
                              <small>其中推理输出</small>
                              <strong>{number(session.usage.reasoningOutput)}</strong>
                            </div>
                          </div>
                          <p className="d-help">
                            取日志中最后一条累计快照；缓存输入和推理输出已包含在输入、输出中。该值不是所选时间窗的新增消耗。
                          </p>
                        </>
                      ) : (
                        <p className="d-help">未读取到用量快照，不根据请求次数估算 token。</p>
                      )}
                    </section>
                    <section className="n-panel">
                      <div className="d-section-title">
                        <h3>会话时间线</h3>
                        <span className="ca-tag">最近事件在前</span>
                      </div>
                      <p className="d-help">
                        合并此会话的请求、重试、回合和工具记录。回合缺少结束记录时，不能据此判断仍在运行。
                      </p>
                      <EventList
                        events={session.events}
                        onOpen={(e) => void openEvidence(e)}
                        onSession={openSession}
                      />
                      {session.hasMore && (
                        <p className="d-help">仅展示最近 1,000 条会话事件，可缩小分析时间范围。</p>
                      )}
                    </section>
                  </>
                )}
              </div>
            </section>
          )}
          {(tab === 'requests' || tab === 'events') && (
            <section className="n-panel">
              <div className="d-section-title">
                <h3>
                  {tab === 'requests' ? '模型 HTTP 尝试' : '合并事件'}
                  {page && <small className="ca-muted"> · {number(page.total)} 条匹配记录</small>}
                </h3>
              </div>
              <div className="ca-filters">
                <label className="ca-search">
                  <Search size={15} />
                  <input
                    aria-label="搜索事件"
                    placeholder="关键词、URL、request-id、会话 ID"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                  />
                </label>
                <select
                  aria-label="筛选日志来源"
                  value={sourceRole}
                  onChange={(e) => setSourceRole(e.target.value)}
                >
                  <option value="">全部运行来源</option>
                  {['runtime', 'cli', 'desktop'].map((r) => (
                    <option key={r} value={r}>
                      {roles[r]}
                    </option>
                  ))}
                </select>
                <select
                  aria-label="筛选会话"
                  value={thread}
                  onChange={(e) => setThread(e.target.value)}
                >
                  <option value="">全部会话</option>
                  {analysis.sessions.map((s) => (
                    <option value={s.id} key={s.id}>
                      {title(s).slice(0, 35)}
                    </option>
                  ))}
                </select>
                {tab === 'events' && (
                  <>
                    <select
                      aria-label="筛选事件类型"
                      value={eventKind}
                      onChange={(e) => setEventKind(e.target.value)}
                    >
                      <option value="">全部类型</option>
                      {Object.entries(kinds)
                        .filter(
                          ([k]) =>
                            ![
                              'turn_started',
                              'turn_completed',
                              'turn_aborted',
                              'context',
                              'compaction',
                              'tool_result',
                            ].includes(k),
                        )
                        .map(([k, label]) => (
                          <option value={k} key={k}>
                            {label}
                          </option>
                        ))}
                    </select>
                    <select
                      aria-label="筛选日志级别"
                      value={level}
                      onChange={(e) => setLevel(e.target.value)}
                    >
                      <option value="">全部级别</option>
                      {['ERROR', 'WARN', 'INFO', 'DEBUG'].map((l) => (
                        <option value={l} key={l}>
                          {l}
                        </option>
                      ))}
                    </select>
                  </>
                )}
                {tab === 'requests' && (
                  <label className="ca-check">
                    <input
                      type="checkbox"
                      checked={onlyFailed}
                      onChange={(e) => setOnlyFailed(e.target.checked)}
                    />
                    只看失败
                  </label>
                )}
                <button
                  className="x-link"
                  onClick={() => {
                    setSearch('');
                    setSourceRole('');
                    setThread('');
                    setEventKind('');
                    setLevel('');
                    setOnlyFailed(false);
                  }}
                >
                  清除筛选
                </button>
              </div>
              <p className="d-help">
                {tab === 'requests'
                  ? '只统计模型接口的 HTTP 尝试；401 / 429 / 5xx 均记为失败。'
                  : '运行数据库与文本日志合并后分页；会话 JSONL 的回合和工具记录在「会话分析」中按需读取。'}{' '}
                所有筛选基于同一次分析快照。
              </p>
              {eventsBusy && !page && (
                <div className="n-progress" role="status">
                  <LoaderCircle size={15} className="n-spin" />
                  正在筛选记录…
                </div>
              )}
              {page && (
                <EventList
                  events={page.events}
                  requests={tab === 'requests'}
                  onOpen={(e) => void openEvidence(e)}
                  onSession={openSession}
                />
              )}
              {page?.nextOffset != null && (
                <div className="ca-load-more">
                  <button
                    className="n-button"
                    disabled={eventsBusy}
                    onClick={() => void loadMore()}
                  >
                    {eventsBusy ? '正在读取…' : '加载更多'}
                  </button>
                  <small>
                    已显示 {page.events.length} / {page.total} 条
                  </small>
                </div>
              )}
            </section>
          )}
          {tab === 'issues' && (
            <section className="n-panel">
              <div className="d-section-title">
                <h3>按故障特征归类</h3>
                <select
                  aria-label="筛选故障类型"
                  value={issueCategory}
                  onChange={(e) => setIssueCategory(e.target.value)}
                >
                  <option value="">全部类型</option>
                  {Object.entries(categories).map(([id, label]) => (
                    <option key={id} value={id}>
                      {label}
                    </option>
                  ))}
                </select>
              </div>
              <p className="d-help">
                相同模块与错误特征合并为一组，计数表示日志条数。同一次故障可能产生请求失败、重试和回合错误多条记录；建议用于定位，实际原因请结合证据确认。
              </p>
              {issues.map((issue) => (
                <IssueCard
                  key={issue.id}
                  issue={issue}
                  onOpen={(id) => void openEvidence(id)}
                  onSession={openSession}
                />
              ))}
              {!issues.length && <div className="ca-empty">已读取范围没有匹配的故障记录。</div>}
            </section>
          )}
          {tab === 'sources' && (
            <>
              <section className="n-panel">
                <div className="d-section-title">
                  <h3>Codex 数据来源</h3>
                  <span className="ca-tag">本机只读</span>
                </div>
                <p className="d-help">
                  主目录：<code>{analysis.codexHome}</code>
                </p>
                <p className="d-help">
                  时间范围：{analysis.since ? date(analysis.since) : '不限起始时间'} —{' '}
                  {date(analysis.generatedAt)}。SQLite 包含尚未合并的 WAL；文本按记录时间筛选。
                </p>
                <div className="ca-source-legend">
                  <span>
                    <Database size={16} />
                    <strong>运行 SQLite</strong>
                    <small>请求、重试与运行事件</small>
                  </span>
                  <span>
                    <Database size={16} />
                    <strong>state SQLite</strong>
                    <small>会话、项目与 provider</small>
                  </span>
                  <span>
                    <FileText size={16} />
                    <strong>CLI / 桌面文本</strong>
                    <small>运行与桌面连接告警</small>
                  </span>
                  <span>
                    <FileText size={16} />
                    <strong>会话 JSONL / zst</strong>
                    <small>回合、工具与累计用量</small>
                  </span>
                </div>
                {analysis.sources.map((s) => (
                  <details
                    key={s.id}
                    className={`ca-source ca-source-${s.status}`}
                    open={s.status === 'error'}
                  >
                    <summary>
                      <span>
                        {s.format === 'SQLite' ? <Database size={16} /> : <FileText size={16} />}
                      </span>
                      <span>
                        <strong>{s.name}</strong>
                        <small>
                          {roles[s.role]} · {s.format}
                          {s.role !== 'rollout' && ` · ${s.records} 条已读记录`}
                        </small>
                      </span>
                      <span
                        className={`ca-tag ${s.status === 'error' || s.partial ? 'ca-tag-warn' : ''}`}
                      >
                        {s.status === 'available'
                          ? s.partial
                            ? '部分读取'
                            : s.role === 'rollout'
                              ? '按需解析'
                              : '已读取'
                          : s.status === 'missing'
                            ? '未发现'
                            : s.status === 'skipped'
                              ? '未选用'
                              : '不可读'}
                      </span>
                      <ChevronRight size={15} className="d-chevron" />
                    </summary>
                    <div>
                      <code>{s.path}</code>
                      <p>{s.note || '在所选时间窗内读取。'}</p>
                      <small>
                        文件 {bytes(s.size)}
                        {s.walSize > 0 && ` · WAL ${bytes(s.walSize)}`} · 修改于{' '}
                        {date(s.modifiedAt)}
                      </small>
                    </div>
                  </details>
                ))}
              </section>
              <section className="n-panel">
                <h3>统计口径</h3>
                <ul className="ca-notes">
                  {analysis.notes.map((note) => (
                    <li key={note}>{note}</li>
                  ))}
                </ul>
                <p className="d-help">显示和导出前隐藏常见凭据；日志始终在本机解析。</p>
              </section>
            </>
          )}
        </>
      )}
      {!analysis && !busy && nativeAvailable && (
        <div className="ca-empty">
          <Activity size={28} />
          <p>尚未读取到分析结果。</p>
          <button className="n-button" onClick={() => void refresh()}>
            重新分析
          </button>
        </div>
      )}
      {(detail || detailBusy) && (
        <Modal
          title="日志证据"
          wide
          onClose={() => {
            detailTicket.current++;
            setDetail(undefined);
            setDetailBusy(false);
          }}
        >
          <div className="ca-evidence">
            {detailBusy && (
              <div className="n-progress" role="status">
                <LoaderCircle size={15} className="n-spin" />
                正在读取证据…
              </div>
            )}
            {detail && (
              <>
                <div className="ca-actions">
                  <span
                    className={`ca-tag ${detail.event.level === 'ERROR' ? 'ca-tag-error' : ''}`}
                  >
                    {detail.event.level}
                  </span>
                  <span>{date(detail.event.time)}</span>
                  <code>{detail.event.target}</code>
                </div>
                <h3>{detail.event.summary}</h3>
                <dl>
                  <dt>来源</dt>
                  <dd>
                    {roles[detail.event.location.sourceRole]}
                    <code>{detail.event.location.path}</code>
                    <small>{detail.event.location.position}</small>
                  </dd>
                  {detail.event.mirrors.map((m) => (
                    <dd key={`${m.sourceId}:${m.position}`}>
                      镜像：<code>{m.path}</code>
                      <small>{m.position}</small>
                    </dd>
                  ))}
                  {detail.event.threadId && (
                    <>
                      <dt>会话 / 回合</dt>
                      <dd>
                        <button
                          className="x-link"
                          onClick={() => {
                            openSession(detail.event.threadId!);
                            setDetail(undefined);
                          }}
                        >
                          {detail.event.threadId}
                        </button>
                        <code>{detail.event.turnId || '回合 ID 未记录'}</code>
                      </dd>
                    </>
                  )}
                  {detail.event.http && (
                    <>
                      <dt>HTTP</dt>
                      <dd>
                        <code>
                          {detail.event.http.method} {detail.event.http.url}
                        </code>
                        <span>
                          状态：{detail.event.http.status ?? '未收到响应'} · request-id：
                          {detail.event.http.requestId ?? '未记录'}
                        </span>
                      </dd>
                    </>
                  )}
                </dl>
                <h4>记录内容</h4>
                <pre>{detail.message}</pre>
                {detail.span && (
                  <details>
                    <summary>跨度上下文</summary>
                    <pre>{detail.span}</pre>
                  </details>
                )}
                <p className="d-help">{detail.note}</p>
                <button
                  className="n-button"
                  onClick={() => {
                    void navigator.clipboard
                      .writeText(
                        `${date(detail.event.time)} ${detail.event.location.path} ${detail.event.location.position}\n${detail.message}`,
                      )
                      .then(() => setNotice('已复制脱敏证据。'))
                      .catch((e) => setError(`复制失败：${reason(e)}`));
                  }}
                >
                  复制脱敏证据
                </button>
              </>
            )}
          </div>
        </Modal>
      )}
    </div>
  );
}
