import { useCallback, useEffect, useRef, useState } from 'react';
import { createPortal } from 'react-dom';
import { Copy, Download, FileText, FolderOpen, Pause, Play, Search, X } from 'lucide-react';
import { diagnosticCall, logsSnapshot } from '../lib/api/diagnostics';
import type { LogSnapshot, ToolboxLog } from '../lib/api/diagnostics';
import { nativeAvailable } from '../lib/api/native';

const levels = { info: '信息', warn: '警告', error: '错误' };
export function LogWindow({ operationId, onClose }: { operationId?: string; onClose: () => void }) {
  const [entries, setEntries] = useState<ToolboxLog[]>([]);
  const [meta, setMeta] = useState<LogSnapshot>();
  const [level, setLevel] = useState('');
  const [query, setQuery] = useState('');
  const [operation, setOperation] = useState(operationId ?? '');
  const [follow, setFollow] = useState(true);
  const [message, setMessage] = useState('');
  const [error, setError] = useState('');
  const [gap, setGap] = useState(false);
  const [target, setTarget] = useState<Element>();
  const list = useRef<HTMLDivElement>(null);
  const closeButton = useRef<HTMLButtonElement>(null);
  const panel = useRef<HTMLElement | null>(null);
  const closeRef = useRef(onClose);
  closeRef.current = onClose;
  const setPanel = useCallback((node: HTMLElement | null) => {
    // 保留最后一个节点，卸载时也能判断焦点是否仍在窗口内。
    if (node) panel.current = node;
  }, []);
  useEffect(() => setOperation(operationId ?? ''), [operationId]);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const locate = () => setTarget(document.querySelector('[role="dialog"]') ?? document.body);
    locate();
    const observer = new MutationObserver(locate);
    observer.observe(document.body, { childList: true, subtree: true });
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') {
        event.preventDefault();
        event.stopImmediatePropagation();
        closeRef.current();
      }
    };
    const outside = (event: MouseEvent) => {
      const origin = event.target;
      if (!(origin instanceof Element)) return;
      // 触发按钮由自身处理开关逻辑，不在此处关闭。
      if (origin.closest('[data-logs-trigger]')) return;
      if (panel.current?.contains(origin)) return;
      closeRef.current();
    };
    document.addEventListener('keydown', escape, true);
    document.addEventListener('mousedown', outside, true);
    return () => {
      observer.disconnect();
      document.removeEventListener('keydown', escape, true);
      document.removeEventListener('mousedown', outside, true);
      const active = document.activeElement;
      if (!active || active === document.body || panel.current?.contains(active)) previous?.focus();
    };
  }, []);
  useEffect(() => {
    if (target) closeButton.current?.focus();
  }, [target]);
  useEffect(() => {
    if (!nativeAvailable) return;
    let active = true;
    let cursor: string | undefined;
    let timer: ReturnType<typeof setTimeout>;
    async function poll() {
      try {
        const snapshot = await logsSnapshot(cursor);
        if (!active) return;
        setMeta(snapshot);
        setError('');
        if (snapshot.gap) setGap(true);
        if (snapshot.entries.length) {
          cursor = snapshot.entries.at(-1)?.id;
          setEntries((current) => {
            const ids = new Set(current.map((v) => v.id));
            return [...current, ...snapshot.entries.filter((v) => !ids.has(v.id))].slice(-1500);
          });
        }
      } catch (reason) {
        if (active) setError(String(reason));
      } finally {
        if (active) timer = setTimeout(() => void poll(), 800);
      }
    }
    void poll();
    return () => {
      active = false;
      clearTimeout(timer);
    };
  }, []);
  useEffect(() => {
    if (follow && list.current) list.current.scrollTop = list.current.scrollHeight;
  }, [entries, follow, query, level, operation]);
  const filtered = entries.filter(
    (entry) =>
      (!level || entry.level === level) &&
      (!operation || entry.operationId === operation) &&
      (!query || `${entry.message} ${entry.source}`.toLowerCase().includes(query.toLowerCase())),
  );
  async function exportLogs() {
    try {
      const result = await diagnosticCall<{ path: string }>('export', {
        operationId: operation || undefined,
      });
      setMessage(`日志已导出：${result.path}`);
      setError('');
    } catch (reason) {
      setError(String(reason));
    }
  }
  if (!target) return null;
  return createPortal(
    <section className="d-log-window" aria-label="工具箱日志窗口" ref={setPanel}>
      <header className="d-log-header">
        <h2>
          <FileText size={17} />
          工具箱日志 <span>{filtered.length} 条</span>
        </h2>
        <div className="d-actions">
          <button className="n-button" onClick={() => setFollow(!follow)} aria-pressed={follow}>
            {follow ? <Pause size={14} /> : <Play size={14} />}
            {follow ? '暂停跟随' : '跟随最新'}
          </button>
          <button className="n-button" disabled={!nativeAvailable} onClick={exportLogs}>
            <Download size={14} />
            导出{operation ? '本次操作' : '最近日志'}
          </button>
          <button
            ref={closeButton}
            className="n-icon-button"
            aria-label="关闭日志窗口"
            onClick={onClose}
          >
            <X size={19} />
          </button>
        </div>
      </header>
      <div className="d-log-toolbar">
        <label className="d-search">
          <Search size={15} />
          <input
            aria-label="搜索工具箱日志"
            placeholder="搜索消息或来源"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
          />
        </label>
        <select aria-label="日志级别" value={level} onChange={(e) => setLevel(e.target.value)}>
          <option value="">全部级别</option>
          <option value="error">错误</option>
          <option value="warn">警告</option>
          <option value="info">信息</option>
        </select>
        {operation && (
          <button className="n-button" onClick={() => setOperation('')}>
            查看全部操作
          </button>
        )}
        <button
          className="n-button"
          onClick={async () => {
            try {
              await navigator.clipboard.writeText(
                filtered
                  .map((v) => `${new Date(v.timestamp).toLocaleString()} [${v.level}] ${v.message}`)
                  .join('\n'),
              );
              setMessage('已复制当前筛选日志');
            } catch {
              setError('复制失败，请使用导出功能。');
            }
          }}
        >
          <Copy size={14} />
          复制
        </button>
        <button className="n-button" onClick={() => setEntries([])}>
          清空视图
        </button>
      </div>
      {(error || meta?.writeError || gap || !!meta?.dropped) && (
        <p className="d-log-notice" role="status">
          {error ||
            meta?.writeError ||
            `部分日志已过期或未落盘（丢弃 ${meta?.dropped ?? 0} 条），当前记录可能不完整。`}
        </p>
      )}
      {message && (
        <p className="d-log-notice" role="status">
          {message}
        </p>
      )}
      <div
        className="d-log-list"
        ref={list}
        onScroll={() => {
          if (
            list.current &&
            list.current.scrollHeight - list.current.scrollTop - list.current.clientHeight > 60
          )
            setFollow(false);
        }}
      >
        {!filtered.length && (
          <p className="d-log-empty">
            {nativeAvailable
              ? '当前筛选下暂无日志。安装、配置和排查操作会记录在这里。'
              : '网页预览不读取本机日志，请使用桌面版。'}
          </p>
        )}
        {filtered.map((entry) => (
          <details key={entry.id} className={`d-log-line d-level-${entry.level}`}>
            <summary>
              <time>
                {new Date(entry.timestamp).toLocaleTimeString('zh-CN', { hour12: false })}
              </time>
              <span className="d-log-level">{levels[entry.level] ?? entry.level}</span>
              <span className="d-log-source">{entry.source}</span>
              <span className="d-log-message">{entry.message}</span>
            </summary>
            <div className="d-log-detail">
              <p>{entry.message}</p>
              <small>
                {new Date(entry.timestamp).toLocaleString('zh-CN')} · 操作{' '}
                {entry.operationId ?? '启动'}
              </small>
              {entry.operationId && (
                <button onClick={() => setOperation(entry.operationId!)}>仅看此操作</button>
              )}
            </div>
          </details>
        ))}
      </div>
      <footer className="d-log-footer">
        <span title={meta?.directory}>{meta?.directory ?? '本地日志 · 已隐藏常见凭据字段'}</span>
        <button
          className="n-button"
          disabled={!nativeAvailable}
          onClick={() => void diagnosticCall('open-logs').catch((e) => setError(String(e)))}
        >
          <FolderOpen size={14} />
          打开日志目录
        </button>
      </footer>
    </section>,
    target,
  );
}
