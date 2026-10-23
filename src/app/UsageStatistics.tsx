import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import {
  BarChart3,
  ChevronLeft,
  ChevronRight,
  Coins,
  Database,
  Download,
  FolderOpen,
  Info,
  Layers,
  LoaderCircle,
  RefreshCw,
  Search,
  Settings2,
  X,
} from 'lucide-react';
import { Modal } from '../components/Primitives';
import { nativeAvailable } from '../lib/api/native';
import { readUsage } from '../lib/api/usage';
import {
  dayKey,
  modelPrice,
  priceFields,
  readPrices,
  referencePrices,
  summarizeUsage,
  usageBounds,
  usageCost,
  usageCsv,
  usageTrend,
  validPrice,
} from '../lib/usage';
import type {
  UsagePrice,
  UsagePrices,
  UsageRange,
  UsageRecord,
  UsageReport,
  UsageWorkspace,
} from '../lib/usage';
import './usage-statistics.css';

const number = (value: number) => value.toLocaleString('zh-CN');
const compact = (value: number) =>
  value >= 1_000_000
    ? `${(value / 1_000_000).toFixed(2)}M`
    : value >= 10_000
      ? `${(value / 1000).toFixed(1)}K`
      : number(value);
const money = (value: number) =>
  `$${value.toLocaleString('en-US', { minimumFractionDigits: 2, maximumFractionDigits: value > 0 && value < 0.01 ? 6 : 4 })}`;
const date = (value: number) => new Date(value).toLocaleString('zh-CN', { hour12: false });
const modelName = (model: string) => (model === 'unknown' ? '未记录模型' : model);
const priceLabels = {
  input: '普通输入',
  output: '输出（含推理）',
  cacheRead: '缓存读取',
  cacheWrite: '缓存写入 · 5 分钟',
  cacheWrite1h: '缓存写入 · 1 小时',
};
// The series colours live in the stylesheet so they follow the appearance.
const colors = [
  'var(--u-series-1)',
  'var(--u-series-2)',
  'var(--u-series-3)',
  'var(--u-series-4)',
  'var(--u-series-5)',
  'var(--u-series-6)',
];
type Tab = 'overview' | 'records' | 'sessions' | 'pricing';
const storageKey = (workspace: UsageWorkspace) => `cyapi.usage.${workspace}`;
function savedSettings(workspace: UsageWorkspace) {
  try {
    const raw = JSON.parse(localStorage.getItem(storageKey(workspace)) ?? '{}');
    const multiplier =
      typeof raw.multiplier === 'number' &&
      Number.isFinite(raw.multiplier) &&
      raw.multiplier >= 0 &&
      raw.multiplier <= 1000
        ? raw.multiplier
        : 1;
    return { prices: readPrices(JSON.stringify(raw.prices ?? {})), multiplier };
  } catch {
    return { prices: {} as UsagePrices, multiplier: 1 };
  }
}

function PriceDialog({
  model,
  initial,
  onSave,
  onClose,
}: {
  model: string;
  initial?: UsagePrice;
  onSave: (price: UsagePrice) => void;
  onClose: () => void;
}) {
  const [error, setError] = useState('');
  return (
    <Modal title={`模型单价 · ${modelName(model)}`} onClose={onClose}>
      <form
        onSubmit={(event) => {
          event.preventDefault();
          const data = new FormData(event.currentTarget);
          const values = Object.fromEntries(
            priceFields.map((key) => [
              key,
              String(data.get(key) ?? '').trim() === '' ? NaN : Number(data.get(key)),
            ]),
          );
          if (!validPrice(values)) {
            setError('请填写 0–1,000,000 之间的有效单价。');
            return;
          }
          onSave(values);
        }}
      >
        <div className="u-price-form">
          <p>单位：USD / 百万 Token。请按所用服务的计费单价填写。</p>
          {priceFields.map((key) => (
            <label key={key}>
              <span>{priceLabels[key]}</span>
              <div>
                <span>$</span>
                <input
                  name={key}
                  type="number"
                  step="any"
                  min="0"
                  max="1000000"
                  required
                  defaultValue={initial?.[key] ?? ''}
                  placeholder="0.00"
                />
              </div>
            </label>
          ))}
          <small>输入已扣除缓存部分，推理输出包含在输出内。单价仅保存在本机当前工作区。</small>
          {error && (
            <p className="u-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="n-dialog-footer">
          <button type="button" className="n-button" onClick={onClose}>
            取消
          </button>
          <button className="n-button n-primary" type="submit">
            保存单价
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function UsageStatistics({ workspace }: { workspace: UsageWorkspace }) {
  const [data, setData] = useState<UsageReport>();
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [range, setRange] = useState<UsageRange>('7d');
  const [start, setStart] = useState(dayKey(Date.now() - 6 * 86400000));
  const [end, setEnd] = useState(dayKey(Date.now()));
  const [tab, setTab] = useState<Tab>('overview');
  const [filterModel, setFilterModel] = useState('');
  const [filterSession, setFilterSession] = useState('');
  const [search, setSearch] = useState('');
  const [page, setPage] = useState(0);
  const [settings, setSettings] = useState(() => savedSettings(workspace));
  const { prices, multiplier } = settings;
  const [multiplierText, setMultiplierText] = useState(String(multiplier));
  const [editingPrice, setEditingPrice] = useState<string>();
  const [metric, setMetric] = useState<'tokens' | 'cost'>('tokens');
  const [hovered, setHovered] = useState<number | null>(null);
  const request = useRef(0);
  const load = useCallback(
    async (refresh = false) => {
      if (!nativeAvailable) return;
      const id = ++request.current;
      setLoading(true);
      setError('');
      try {
        const report = await readUsage(workspace, refresh);
        if (id === request.current) setData(report);
      } catch (e) {
        if (id === request.current) setError(e instanceof Error ? e.message : String(e));
      } finally {
        if (id === request.current) setLoading(false);
      }
    },
    [workspace],
  );
  useEffect(() => {
    void load();
    return () => {
      request.current++;
    };
  }, [load]);
  useEffect(() => {
    setPage(0);
    setHovered(null);
  }, [range, start, end, filterModel, filterSession, search, tab, data]);
  function saveSettings(next: typeof settings) {
    try {
      localStorage.setItem(storageKey(workspace), JSON.stringify(next));
      setSettings(next);
      setError('');
      return true;
    } catch {
      setError('本机设置未能保存，请检查磁盘空间或存储权限。');
      return false;
    }
  }
  const bounds = usageBounds(range, start, end);
  const allRecords = data?.records ?? [];
  const models = useMemo(() => [...new Set(allRecords.map((r) => r.model))].sort(), [allRecords]);
  const priceModels = useMemo(
    () => [...new Set([...models, ...Object.keys(prices)])].sort(),
    [models, prices],
  );
  const records = useMemo(() => {
    const query = search.trim().toLowerCase();
    return allRecords.filter(
      (r) =>
        r.time >= bounds.from &&
        r.time < bounds.to &&
        (!filterModel || r.model === filterModel) &&
        (!filterSession || r.sessionId === filterSession) &&
        (!query || `${r.model} ${r.project} ${r.sessionId}`.toLowerCase().includes(query)),
    );
  }, [allRecords, bounds.from, bounds.to, filterModel, filterSession, search]);
  const totals = useMemo(
    () => summarizeUsage(records, prices, multiplier),
    [records, prices, multiplier],
  );
  const trend = useMemo(
    () => usageTrend(records, prices, multiplier, bounds.from, bounds.to),
    [records, prices, multiplier, bounds.from, bounds.to],
  );
  const groups = useMemo(() => {
    const byModel = new Map<string, UsageRecord[]>();
    const bySession = new Map<string, UsageRecord[]>();
    for (const record of records) {
      const m = byModel.get(record.model) ?? [];
      m.push(record);
      byModel.set(record.model, m);
      const s = bySession.get(record.sessionId) ?? [];
      s.push(record);
      bySession.set(record.sessionId, s);
    }
    return {
      models: [...byModel]
        .map(([model, rows]) => ({ model, ...summarizeUsage(rows, prices, multiplier) }))
        .sort((a, b) => b.input + b.output - a.input - a.output),
      sessions: [...bySession]
        .map(([id, rows]) => ({
          id,
          project: rows.find((r) => r.project)?.project ?? '',
          time: rows.reduce((time, r) => Math.max(time, r.time), 0),
          models: [...new Set(rows.map((r) => r.model))],
          ...summarizeUsage(rows, prices, multiplier),
        }))
        .sort((a, b) => b.time - a.time),
    };
  }, [records, prices, multiplier]);
  const pages = Math.max(
    1,
    Math.ceil((tab === 'sessions' ? groups.sessions.length : records.length) / 25),
  );
  const currentPage = Math.min(page, pages - 1);
  const tokenTotal = totals.input + totals.output;
  const maxBar = Math.max(
    1,
    ...trend.map((b) => (metric === 'tokens' ? b.input + b.output : b.cost)),
  );
  const bucket = hovered === null ? undefined : trend[hovered];
  const costText = (cost: number, unpriced: number, count: number) =>
    count > 0 && unpriced === count ? '待配置' : money(cost);
  const exportCsv = () => {
    const url = URL.createObjectURL(
      new Blob([usageCsv(records, prices, multiplier)], { type: 'text/csv;charset=utf-8;' }),
    );
    const link = document.createElement('a');
    link.href = url;
    link.download = `cyapi-${workspace}-usage-${dayKey(Date.now())}.csv`;
    document.body.appendChild(link);
    link.click();
    link.remove();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  const openSession = (session: string) => {
    setFilterSession(session);
    setTab('records');
  };
  const pagination = (
    <div className="u-pagination">
      <span>
        共 {number(tab === 'sessions' ? groups.sessions.length : records.length)} 条 · 每页 25 条
      </span>
      <div>
        <button
          className="n-icon-button"
          disabled={currentPage === 0}
          aria-label="上一页"
          onClick={() => setPage(currentPage - 1)}
        >
          <ChevronLeft size={16} />
        </button>
        <span>
          {currentPage + 1} / {pages}
        </span>
        <button
          className="n-icon-button"
          disabled={currentPage + 1 >= pages}
          aria-label="下一页"
          onClick={() => setPage(currentPage + 1)}
        >
          <ChevronRight size={16} />
        </button>
      </div>
    </div>
  );

  return (
    <section className="u-page" aria-label="用量统计分析" aria-busy={loading}>
      <div className="u-heading u-heading-compact">
        <div className="u-actions">
          <button
            className="n-button"
            disabled={!records.length || loading || !!bounds.error}
            onClick={exportCsv}
          >
            <Download size={14} />
            导出 CSV
          </button>
          <button
            className="n-button n-primary"
            disabled={!nativeAvailable || loading}
            onClick={() => void load()}
          >
            <RefreshCw size={14} className={loading ? 'n-spin' : ''} />
            {loading ? '读取中…' : '刷新统计'}
          </button>
        </div>
      </div>
      {error && (
        <div className="u-notice u-error" role="alert">
          {error}
          <button className="u-text-button" disabled={loading} onClick={() => void load()}>
            重试
          </button>
        </div>
      )}
      {!nativeAvailable && (
        <div className="u-notice">
          <Info size={16} />
          <span>请在桌面版查看本机用量。支持 Codex、Claude Code 和 OpenCode 的历史会话记录。</span>
        </div>
      )}
      {data?.partial && (
        <div className="u-notice u-warning" role="status">
          <Info size={16} />
          <span>
            部分来源未能完整读取，当前结果可能不完整。展开底部「数据来源与统计口径」查看详情。
          </span>
        </div>
      )}
      <div className="u-toolbar">
        <div className="u-ranges" role="group" aria-label="统计时间范围">
          {(
            [
              ['today', '今天'],
              ['7d', '近 7 天'],
              ['30d', '近 30 天'],
              ['all', '全部'],
              ['custom', '自定义'],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              aria-pressed={range === id}
              className={range === id ? 'active' : ''}
              onClick={() => setRange(id)}
            >
              {label}
            </button>
          ))}
        </div>
        <label className="u-model-select">
          <span>模型</span>
          <select
            aria-label="筛选模型"
            value={filterModel}
            onChange={(e) => setFilterModel(e.target.value)}
          >
            <option value="">全部模型</option>
            {models.map((model) => (
              <option key={model} value={model}>
                {modelName(model)}
              </option>
            ))}
          </select>
        </label>
      </div>
      {range === 'custom' && (
        <div className="u-dates">
          <label>
            开始日期
            <input type="date" value={start} onChange={(e) => setStart(e.target.value)} />
          </label>
          <span>至</span>
          <label>
            结束日期
            <input type="date" value={end} onChange={(e) => setEnd(e.target.value)} />
          </label>
          {bounds.error && (
            <span role="alert" className="u-error">
              {bounds.error}
            </span>
          )}
        </div>
      )}
      <div className="u-metrics">
        <article>
          <span className="u-metric-label">
            <Layers size={15} />总 Token
          </span>
          <strong title={number(tokenTotal)}>{compact(tokenTotal)}</strong>
          <small>
            输入 {compact(totals.input)} <span>·</span> 输出 {compact(totals.output)}
          </small>
        </article>
        <article className="u-cost-card">
          <span className="u-metric-label">
            <Coins size={15} />
            估算费用 <i>USD</i>
          </span>
          <strong>{costText(totals.cost, totals.unpriced, totals.count)}</strong>
          <small>
            {totals.unpriced > 0 ? (
              <button onClick={() => setTab('pricing')}>
                {number(totals.unpriced)} 条待配置单价 <ChevronRight size={12} />
              </button>
            ) : (
              `按单价 / 记录费用估算 · ${multiplier}×`
            )}
          </small>
        </article>
        <article>
          <span className="u-metric-label">
            <Database size={15} />
            缓存读取占比
          </span>
          <strong>
            {totals.input ? `${((totals.cached / totals.input) * 100).toFixed(1)}%` : '—'}
          </strong>
          <small>缓存读取 {compact(totals.cached)} Token</small>
        </article>
        <article>
          <span className="u-metric-label">
            <BarChart3 size={15} />
            计费用量记录
          </span>
          <strong>{number(totals.count)}</strong>
          <small>覆盖 {number(totals.sessions)} 个会话</small>
        </article>
      </div>
      <div className="u-tab-row">
        <nav className="u-tabs" aria-label="用量统计视图">
          {(
            [
              ['overview', '用量总览'],
              ['records', '用量明细'],
              ['sessions', '会话分析'],
              ['pricing', '模型单价'],
            ] as const
          ).map(([id, label]) => (
            <button
              key={id}
              className={tab === id ? 'active' : ''}
              aria-current={tab === id ? 'page' : undefined}
              onClick={() => setTab(id)}
            >
              {label}
            </button>
          ))}
        </nav>
        {tab !== 'pricing' && (
          <label className="u-search">
            <Search size={14} />
            <input
              aria-label="搜索用量记录"
              placeholder="搜索模型、项目、会话…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </label>
        )}
      </div>
      {filterSession && tab !== 'pricing' && (
        <div className="u-filter-chip">
          会话 {filterSession}
          <button onClick={() => setFilterSession('')} aria-label="清除会话筛选">
            <X size={13} />
          </button>
        </div>
      )}
      {loading && !data && (
        <div className="u-empty" role="status">
          <LoaderCircle size={26} className="n-spin" />
          <strong>正在读取本机会话用量</strong>
          <p>首次扫描可能需要一些时间，后续刷新会复用未变化的文件。</p>
        </div>
      )}
      {!loading && !records.length && tab !== 'pricing' && (
        <div className="u-empty">
          <BarChart3 size={30} />
          <strong>{allRecords.length ? '没有符合筛选条件的用量' : '尚未发现用量记录'}</strong>
          <p>
            {allRecords.length
              ? '调整时间范围、模型或搜索条件后再查看。'
              : '使用当前工作区的 Agent 完成会话后，点击「刷新统计」。'}
          </p>
        </div>
      )}
      {tab === 'overview' && records.length > 0 && (
        <>
          <div className="u-chart-grid">
            <article className="u-panel u-trend">
              <div className="u-panel-heading">
                <div>
                  <h3>用量趋势</h3>
                  <p>
                    {bucket
                      ? `${bucket.start}${bucket.end !== bucket.start ? ` — ${bucket.end}` : ''} · ${number(bucket.count)} 条 · ${metric === 'tokens' ? `${number(bucket.input + bucket.output)} Token` : money(bucket.cost)}`
                      : '按本机日期汇总 · 悬停查看详情'}
                  </p>
                </div>
                <div className="u-segment">
                  <button
                    className={metric === 'tokens' ? 'active' : ''}
                    onClick={() => setMetric('tokens')}
                  >
                    Token
                  </button>
                  <button
                    className={metric === 'cost' ? 'active' : ''}
                    onClick={() => setMetric('cost')}
                  >
                    费用
                  </button>
                </div>
              </div>
              <div className="u-chart">
                <div className="u-chart-axis">
                  <span>{metric === 'tokens' ? compact(maxBar) : money(maxBar)}</span>
                  <span>{metric === 'tokens' ? compact(maxBar / 2) : money(maxBar / 2)}</span>
                  <span>0</span>
                </div>
                <div className="u-chart-plot">
                  {trend.map((b, i) => (
                    <button
                      key={b.start}
                      className={hovered === i ? 'selected' : ''}
                      onMouseEnter={() => setHovered(i)}
                      onMouseLeave={() => setHovered(null)}
                      onFocus={() => setHovered(i)}
                      onBlur={() => setHovered(null)}
                      aria-label={`${b.start} 至 ${b.end}，输入 ${number(b.input)}，输出 ${number(b.output)}，已估算 ${money(b.cost)}，${b.unpriced} 条待配置单价`}
                      title={`${b.start}${b.end !== b.start ? ` — ${b.end}` : ''}\n输入 ${number(b.input)} / 输出 ${number(b.output)}\n${money(b.cost)} · ${b.count} 条${b.unpriced ? ` · ${b.unpriced} 条待定价` : ''}`}
                    >
                      <span
                        className="u-chart-stack"
                        style={{
                          height: `${((metric === 'tokens' ? b.input + b.output : b.cost) / maxBar) * 100}%`,
                        }}
                      >
                        {metric === 'tokens' ? (
                          <>
                            <i className="u-bar-output" style={{ flex: b.output }} />
                            <i className="u-bar-input" style={{ flex: b.input }} />
                          </>
                        ) : (
                          <i className="u-bar-input" style={{ flex: 1 }} />
                        )}
                      </span>
                      {(i === 0 ||
                        i === trend.length - 1 ||
                        i % Math.max(1, Math.ceil(trend.length / 5)) === 0) && (
                        <small>{b.start.slice(5).replace('-', '/')}</small>
                      )}
                    </button>
                  ))}
                </div>
              </div>
              <div className="u-chart-legend">
                {metric === 'tokens' ? (
                  <>
                    <span>
                      <i className="u-bar-input" />
                      输入（含缓存）
                    </span>
                    <span>
                      <i className="u-bar-output" />
                      输出（含推理）
                    </span>
                  </>
                ) : (
                  <span>仅汇总已定价记录 · USD</span>
                )}
              </div>
            </article>
            <article className="u-panel u-distribution">
              <div className="u-panel-heading">
                <div>
                  <h3>模型分布</h3>
                  <p>按 Token 消耗排序</p>
                </div>
                <span>{groups.models.length} 个模型</span>
              </div>
              <div className="u-distribution-bar" aria-hidden="true">
                {groups.models.slice(0, 5).map((m, i) => (
                  <i key={m.model} style={{ flex: m.input + m.output, background: colors[i] }} />
                ))}
                {groups.models.length > 5 && (
                  <i
                    style={{
                      flex: groups.models.slice(5).reduce((n, m) => n + m.input + m.output, 0),
                      background: colors[5],
                    }}
                  />
                )}
              </div>
              <div className="u-model-list">
                {groups.models.slice(0, 5).map((m, i) => (
                  <button key={m.model} onClick={() => setFilterModel(m.model)} title={m.model}>
                    <i style={{ background: colors[i] }} />
                    <span>
                      {modelName(m.model)}
                      <small>{compact(m.input + m.output)} Token</small>
                    </span>
                    <strong>{(((m.input + m.output) / tokenTotal) * 100).toFixed(1)}%</strong>
                  </button>
                ))}
              </div>
              {groups.models.length > 5 && (
                <small>另有 {groups.models.length - 5} 个模型，见下方明细</small>
              )}
            </article>
          </div>
          <article className="u-panel">
            <div className="u-panel-heading">
              <div>
                <h3>模型用量</h3>
                <p>缓存和推理 Token 已包含在输入 / 输出中</p>
              </div>
              <button className="u-text-button" onClick={() => setTab('pricing')}>
                <Settings2 size={14} />
                配置单价
              </button>
            </div>
            <div className="u-table-scroll">
              <table className="u-table">
                <thead>
                  <tr>
                    <th>模型</th>
                    <th>记录数</th>
                    <th>输入 Token</th>
                    <th>输出 Token</th>
                    <th>缓存读取</th>
                    <th>估算费用</th>
                  </tr>
                </thead>
                <tbody>
                  {groups.models.map((m) => (
                    <tr key={m.model}>
                      <td>
                        <button
                          className="u-text-button"
                          onClick={() => {
                            setFilterModel(m.model);
                            setTab('records');
                          }}
                        >
                          {modelName(m.model)}
                        </button>
                      </td>
                      <td>{number(m.count)}</td>
                      <td>{number(m.input)}</td>
                      <td>{number(m.output)}</td>
                      <td>{number(m.cached)}</td>
                      <td>
                        {costText(m.cost, m.unpriced, m.count)}
                        {m.unpriced > 0 && m.unpriced < m.count && (
                          <small>{m.unpriced} 条待定价</small>
                        )}
                      </td>
                    </tr>
                  ))}
                </tbody>
              </table>
            </div>
          </article>
        </>
      )}
      {tab === 'records' && records.length > 0 && (
        <article className="u-panel">
          <div className="u-table-scroll">
            <table className="u-table u-record-table">
              <thead>
                <tr>
                  <th>时间 / 模型</th>
                  <th>会话</th>
                  <th>输入 / 输出</th>
                  <th>缓存读 / 写</th>
                  <th>估算费用</th>
                </tr>
              </thead>
              <tbody>
                {records.slice(currentPage * 25, currentPage * 25 + 25).map((r) => {
                  const cost = usageCost(r, prices, multiplier);
                  return (
                    <tr key={r.id}>
                      <td>
                        <time>{date(r.time)}</time>
                        <small title={r.model}>{modelName(r.model)}</small>
                      </td>
                      <td>
                        <button
                          className="u-text-button u-mono"
                          title={r.sessionId}
                          onClick={() => openSession(r.sessionId)}
                        >
                          {r.sessionId.slice(0, 12)}
                        </button>
                        {r.project && (
                          <small title={r.project}>
                            {r.project.split(/[\\/]/).filter(Boolean).at(-1)}
                          </small>
                        )}
                      </td>
                      <td>
                        {number(r.input)}
                        <small>
                          {number(r.output)}
                          {r.reasoningOutput > 0 && `（含推理 ${compact(r.reasoningOutput)}）`}
                        </small>
                      </td>
                      <td>
                        {number(r.cachedInput)}
                        <small>{number(r.cacheWrite)}</small>
                      </td>
                      <td>
                        {cost.usd === null ? (
                          <button
                            className="u-text-button"
                            onClick={() => setEditingPrice(r.model)}
                          >
                            配置单价
                          </button>
                        ) : (
                          <>
                            {money(cost.usd)}
                            <small>
                              {cost.kind === 'reported'
                                ? '记录费用'
                                : cost.kind === 'reference'
                                  ? '参考单价'
                                  : '自定义单价'}
                            </small>
                          </>
                        )}
                      </td>
                    </tr>
                  );
                })}
              </tbody>
            </table>
          </div>
          {pagination}
        </article>
      )}
      {tab === 'sessions' && groups.sessions.length > 0 && (
        <article className="u-panel">
          <div className="u-table-scroll">
            <table className="u-table">
              <thead>
                <tr>
                  <th>会话 / 最近用量</th>
                  <th>模型 / 项目</th>
                  <th>记录数</th>
                  <th>总 Token</th>
                  <th>估算费用</th>
                  <th />
                </tr>
              </thead>
              <tbody>
                {groups.sessions.slice(currentPage * 25, currentPage * 25 + 25).map((s) => (
                  <tr key={s.id}>
                    <td className="u-mono" title={s.id}>
                      {s.id.slice(0, 16)}
                      <small>{date(s.time)}</small>
                    </td>
                    <td>
                      <span title={s.models.join(', ')}>
                        {modelName(s.models[0])}
                        {s.models.length > 1 && ` +${s.models.length - 1}`}
                      </span>
                      <small title={s.project}>
                        {s.project.split(/[\\/]/).filter(Boolean).at(-1) || '未记录项目'}
                      </small>
                    </td>
                    <td>{number(s.count)}</td>
                    <td>{number(s.input + s.output)}</td>
                    <td>{costText(s.cost, s.unpriced, s.count)}</td>
                    <td>
                      <button className="u-text-button" onClick={() => openSession(s.id)}>
                        查看记录
                        <ChevronRight size={13} />
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
          {pagination}
        </article>
      )}
      {tab === 'pricing' && (
        <article className="u-panel">
          <div className="u-panel-heading">
            <div>
              <h3>模型单价</h3>
              <p>USD / 百万 Token · 自定义单价优先于记录费用及参考单价</p>
            </div>
          </div>
          <div className="u-pricing-note">
            <Info size={15} />
            <p>
              内置价格仅供参考。中转站、长上下文、订阅优惠等计费规则请按实际服务调整。未定价记录保留
              Token，用量费用显示为待配置。
            </p>
          </div>
          <form
            className="u-multiplier"
            onSubmit={(e) => {
              e.preventDefault();
              const value = multiplierText.trim() === '' ? NaN : Number(multiplierText);
              if (!Number.isFinite(value) || value < 0 || value > 1000) {
                setError('费用倍率需要是 0–1000 之间的数值。');
                return;
              }
              saveSettings({ prices, multiplier: value });
            }}
          >
            <label htmlFor="usage-multiplier">费用倍率</label>
            <input
              id="usage-multiplier"
              type="number"
              min="0"
              max="1000"
              step="any"
              required
              value={multiplierText}
              onChange={(e) => setMultiplierText(e.target.value)}
            />
            <button className="n-button" type="submit">
              应用倍率
            </button>
            <small>当前 {multiplier}× · 对当前工作区的全部费用生效</small>
          </form>
          {!priceModels.length ? (
            <div className="u-empty">
              <Coins size={28} />
              <strong>读取用量后即可配置模型单价</strong>
              <p>模型将按会话中记录的名称自动列出。</p>
            </div>
          ) : (
            <div className="u-table-scroll">
              <table className="u-table u-price-table">
                <thead>
                  <tr>
                    <th>模型 / 来源</th>
                    {priceFields.map((key) => (
                      <th key={key}>{priceLabels[key]}</th>
                    ))}
                    <th />
                  </tr>
                </thead>
                <tbody>
                  {priceModels.map((model) => {
                    const p = modelPrice(model, prices);
                    return (
                      <tr key={model}>
                        <td>
                          {modelName(model)}
                          <small>
                            {Object.hasOwn(prices, model)
                              ? '自定义'
                              : Object.hasOwn(referencePrices, model)
                                ? '参考单价'
                                : '待配置'}
                          </small>
                        </td>
                        {priceFields.map((key) => (
                          <td key={key}>{p ? `$${p[key]}` : '—'}</td>
                        ))}
                        <td>
                          <div className="u-price-actions">
                            <button
                              className="u-text-button"
                              onClick={() => setEditingPrice(model)}
                            >
                              {p ? '编辑' : '配置'}
                            </button>
                            {Object.hasOwn(prices, model) && (
                              <button
                                className="u-text-button u-muted"
                                onClick={() => {
                                  const next = { ...prices };
                                  delete next[model];
                                  saveSettings({ prices: next, multiplier });
                                }}
                              >
                                移除自定义
                              </button>
                            )}
                          </div>
                        </td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
            </div>
          )}
        </article>
      )}
      <details className="u-sources">
        <summary>
          <FolderOpen size={15} />
          数据来源与统计口径
          <span>
            {data
              ? `${data.sources.reduce((n, s) => n + s.files, 0)} 份文件 · ${data.duplicates} 条重复已合并`
              : '只读本地记录'}
          </span>
        </summary>
        <div>
          <p>
            统计仅包含已落盘且带 Token
            的模型响应，不包含没有用量的失败请求。输入包含缓存读写，输出包含推理；费用是估算，不等同于账户账单或余额。近
            7 / 30 天包含今天，日期按本机时区计算。
          </p>
          <p>
            Codex 读取 sessions 与 archived_sessions；Claude Code 读取 projects（含子
            Agent）；OpenCode 优先读取数据库及 WAL，旧版读取
            storage/message。仅返回用量元数据，不读取凭据，不上传会话正文。
          </p>
          {data?.sources.map((source) => (
            <div className="u-source" key={source.path}>
              <div>
                <code>{source.path}</code>
                <span>
                  {source.status === 'missing'
                    ? '目录不存在'
                    : source.status === 'error'
                      ? '读取失败'
                      : `${source.files} 份文件 · ${source.cachedFiles} 份复用缓存`}
                </span>
              </div>
              {source.notes.map((note, i) => (
                <p className="u-error" key={i}>
                  {note}
                </p>
              ))}
            </div>
          ))}
          {data?.notes.map((note) => (
            <p key={note} className="u-error">
              {note}
            </p>
          ))}
          <div className="u-source-footer">
            <span>{data ? `更新于 ${date(data.generatedAt)}` : '尚未读取本机记录'}</span>
            <button
              className="u-text-button"
              disabled={!nativeAvailable || loading}
              onClick={() => void load(true)}
            >
              重新扫描全部文件
            </button>
          </div>
        </div>
      </details>
      {editingPrice !== undefined && (
        <PriceDialog
          model={editingPrice}
          initial={modelPrice(editingPrice, prices)}
          onClose={() => setEditingPrice(undefined)}
          onSave={(price) => {
            if (saveSettings({ prices: { ...prices, [editingPrice]: price }, multiplier }))
              setEditingPrice(undefined);
          }}
        />
      )}
    </section>
  );
}
