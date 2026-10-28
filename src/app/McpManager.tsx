import { useEffect, useRef, useState } from 'react';
import {
  ArrowRight,
  CheckCircle2,
  CircleHelp,
  Globe2,
  LoaderCircle,
  Package,
  Plug,
  Plus,
  Power,
  PowerOff,
  RefreshCw,
  Search,
  Trash2,
  Unplug,
} from 'lucide-react';
import { nativeAvailable, nativeCall, SERVER_URL } from '../lib/api/native';
import { mcpConfirmation } from '../features/mcp/actions';
import type {
  McpCatalog,
  McpDescriptor,
  McpEntryStatus,
  McpLocalEntry,
  McpView,
} from '../lib/api/native';

type Workspace = McpView['workspace'];
type McpTab = 'catalog' | 'installed';
const workspaceNames: Record<Workspace, string> = {
  codex: 'Codex',
  claude: 'Claude Code',
  opencode: 'OpenCode',
};
const workspaceNotes: Record<Workspace, string> = {
  codex: '管理 Codex 的 MCP 接入；停用保留配置，移除接入不会卸载服务。',
  claude: '管理 Claude Code 的全局 MCP 接入；项目内启用或停用请在 /mcp 中操作。',
  opencode: '管理 OpenCode 的 MCP 接入；停用保留配置，移除接入不会卸载服务。',
};
const errorText = (error: unknown) => (error instanceof Error ? error.message : String(error));

type LocalForm = {
  id: string;
  name: string;
  description: string;
  transportType: string;
  command: string;
  args: string;
  url: string;
  codex: boolean;
  claude: boolean;
  opencode: boolean;
};
const emptyLocalForm = (workspace: Workspace): LocalForm => ({
  id: '',
  name: '',
  description: '',
  transportType: 'stdio',
  command: '',
  args: '',
  url: '',
  codex: workspace === 'codex',
  claude: workspace === 'claude',
  opencode: workspace === 'opencode',
});

function transportLabel(transport: string) {
  return transport === 'http' ? 'HTTP' : transport === 'stdio' ? 'stdio' : transport;
}

type ActionHandler = (action: string, serverId: string) => void;

function InstalledActions({
  entry,
  server,
  status,
  workspace,
  disabled,
  onAction,
}: {
  entry: McpLocalEntry;
  server?: McpDescriptor;
  status?: McpEntryStatus;
  workspace: Workspace;
  disabled: boolean;
  onAction: ActionHandler;
}) {
  const canUpdate = server && status?.canInstall && entry.managed && !status.recognized;
  if (entry.readOnlyReason)
    return (
      <span className="n-config-note" title={entry.readOnlyReason}>
        只读
      </span>
    );
  return (
    <div className="n-mcp-installed-actions">
      {server && (canUpdate || status?.state === 'stale') && (
        <button
          className="n-button"
          disabled={disabled || !status?.tokenAvailable}
          onClick={() => onAction('mcp-install', server.id)}
        >
          <RefreshCw size={14} />
          {status?.state === 'stale' ? '更新凭证' : '更新配置'}
        </button>
      )}
      {workspace !== 'claude' && (
        <button
          className="n-button"
          disabled={disabled}
          onClick={() => onAction(entry.enabled ? 'mcp-disable' : 'mcp-enable', entry.name)}
        >
          {entry.enabled ? <PowerOff size={14} /> : <Power size={14} />}
          {entry.enabled ? '停用' : '启用'}
        </button>
      )}
      <button
        className="n-button"
        disabled={disabled}
        onClick={() => onAction('mcp-remove', entry.name)}
        aria-label={`从 ${workspaceNames[workspace]} 移除 ${entry.name} 的接入`}
      >
        <Unplug size={14} /> 移除接入
      </button>
    </div>
  );
}

type McpFocus = { query: string; tab: McpTab };
export function McpManager({ workspace, focus }: { workspace: Workspace; focus?: McpFocus }) {
  const [tab, setTab] = useState<McpTab>(focus?.tab ?? 'catalog');
  // Keep the tab, but discard requests, forms and snapshots when switching software.
  return (
    <WorkspaceMcpManager
      key={workspace}
      workspace={workspace}
      tab={tab}
      onTab={setTab}
      focus={focus}
    />
  );
}

function WorkspaceMcpManager({
  workspace,
  tab,
  onTab,
  focus,
}: {
  workspace: Workspace;
  tab: McpTab;
  onTab: (tab: McpTab) => void;
  focus?: McpFocus;
}) {
  const [view, setView] = useState<McpView>();
  const [busy, setBusy] = useState('read');
  const [catalogBusy, setCatalogBusy] = useState(false);
  const [catalogError, setCatalogError] = useState('');
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [query, setQuery] = useState(focus?.query ?? '');
  const [transport, setTransport] = useState('');
  const [localForm, setLocalForm] = useState<LocalForm>(() => emptyLocalForm(workspace));
  const mounted = useRef(false);
  const localVersion = useRef(0);
  const agentName = workspaceNames[workspace];

  function browserView(catalog: McpCatalog): McpView {
    return {
      catalog,
      workspace,
      configPath: '',
      revision: '',
      source: 'browser',
      entries: [],
      local: [],
    };
  }

  async function readView(): Promise<McpView> {
    if (nativeAvailable) return nativeCall<McpView>('mcp-view', { workspace });
    return browserView({ schema: 2, generatedAt: '', servers: [] });
  }

  async function refreshCatalog(force = true) {
    const version = localVersion.current;
    setCatalogBusy(true);
    setCatalogError('');
    try {
      let next: McpView;
      if (nativeAvailable) {
        next = await nativeCall<McpView>(force ? 'mcp-refresh' : 'mcp-catalog-read', { workspace });
      } else {
        const response = await fetch(`${SERVER_URL}/api/mcp`, {
          signal: AbortSignal.timeout(15_000),
        });
        if (!response.ok) throw new Error('MCP 广场暂不可用，请稍后重试。');
        const catalog = (await response.json()) as McpCatalog;
        if (![1, 2].includes(catalog.schema) || !Array.isArray(catalog.servers))
          throw new Error('请更新服务端 MCP 目录。');
        next = browserView(catalog);
      }
      if (!mounted.current) return;
      if (nativeAvailable && version !== localVersion.current) {
        // A local operation finished while the catalog was being fetched. Read
        // its current result instead of replacing it with the older snapshot.
        const currentVersion = localVersion.current;
        next = await readView();
        if (!mounted.current || currentVersion !== localVersion.current) return;
      }
      setView(next);
    } catch (error) {
      if (mounted.current) setCatalogError(errorText(error));
    } finally {
      if (mounted.current) setCatalogBusy(false);
    }
  }

  useEffect(() => {
    let active = true;
    mounted.current = true;
    void readView()
      .then((next) => {
        if (!active) return;
        setView(next);
        void refreshCatalog(false);
      })
      .catch((error) => {
        if (active) setNotice({ text: errorText(error), error: true });
      })
      .finally(() => {
        if (active) setBusy('');
      });
    return () => {
      active = false;
      mounted.current = false;
    };
  }, []);

  // An action only writes the entry it names, so it carries the revision of
  // that entry. A background rewrite of the agent configuration, for example
  // the pipe of a restarted runtime, must not invalidate a pending action.
  function revisionOf(action: string, serverId?: string) {
    if (!serverId) return view?.revision;
    if (action === 'mcp-install')
      return view?.entries.find((entry) => entry.id === serverId)?.revision;
    return view?.local.find((entry) => entry.name === serverId)?.revision;
  }

  async function update(action: string, serverId?: string) {
    const confirmation = mcpConfirmation(
      action,
      workspace,
      view?.local.find((entry) => entry.name === serverId),
    );
    if (confirmation && !window.confirm(confirmation)) return;
    setBusy(action);
    setNotice(undefined);
    try {
      const next = await nativeCall<McpView>(action, {
        workspace,
        serverId,
        configurationRevision: revisionOf(action, serverId),
      });
      if (!mounted.current) return;
      localVersion.current++;
      setView(next);
      const messages: Record<string, string> = {
        'mcp-view': `已重新检测 ${agentName} 的 MCP 配置。`,
        'mcp-install':
          workspace === 'opencode'
            ? '已接入 OpenCode，配置会自动重新加载。'
            : `已接入 ${agentName}，重启 ${agentName} 后生效。`,
        'mcp-remove': `已从 ${agentName} 移除 MCP 接入，未卸载服务。`,
        'mcp-enable': `已启用 ${agentName} 的 MCP 配置。`,
        'mcp-disable': `已停用 ${agentName} 的 MCP 配置。`,
        'mcp-local-remove': '已删除本机自定义条目，已接入的软件配置仍保留。',
      };
      setNotice({ text: messages[action] });
    } catch (error) {
      if (mounted.current) setNotice({ text: errorText(error), error: true });
    } finally {
      if (mounted.current) setBusy('');
    }
  }

  async function saveLocal() {
    setBusy('mcp-local-save');
    setNotice(undefined);
    try {
      const next = await nativeCall<McpView>('mcp-local-save', {
        workspace,
        values: localForm as unknown as Record<string, unknown>,
      });
      if (!mounted.current) return;
      localVersion.current++;
      setView(next);
      setNotice({ text: '已保存本机条目，可在 MCP 广场中选择对应软件接入。' });
      setLocalForm(emptyLocalForm(workspace));
    } catch (error) {
      if (mounted.current) setNotice({ text: errorText(error), error: true });
    } finally {
      if (mounted.current) setBusy('');
    }
  }

  const statusOf = (id: string) => view?.entries.find((entry) => entry.id === id);
  const matches = (...text: string[]) =>
    text.join(' ').toLowerCase().includes(query.trim().toLowerCase());
  const servers = (view?.catalog.servers ?? []).filter((server) =>
    server.workspaces.includes(workspace),
  );
  const catalogEntries = servers.filter(
    (server) =>
      (!transport || server.transport.type === transport) &&
      matches(server.id, server.name, server.description, ...server.capabilities),
  );
  const installedEntries = (view?.local ?? []).filter((entry) => {
    const server = view?.catalog.servers.find((server) => server.id === entry.catalogId);
    return (
      (!transport || entry.transport === transport) &&
      matches(entry.name, entry.target, server?.name ?? '')
    );
  });
  const disabled = !!busy || !view || !nativeAvailable;
  const filtered = !!query.trim() || !!transport;
  const onAction: ActionHandler = (action, id) => void update(action, id);

  function switchTab(next: McpTab) {
    onTab(next);
    setQuery('');
    setTransport('');
  }

  return (
    <div className="n-mcp">
      <nav className="n-skill-tabs" aria-label="MCP 管理视图">
        <button
          className={tab === 'catalog' ? 'active' : ''}
          aria-current={tab === 'catalog' ? 'page' : undefined}
          onClick={() => switchTab('catalog')}
        >
          MCP 广场
        </button>
        <button
          className={tab === 'installed' ? 'active' : ''}
          aria-current={tab === 'installed' ? 'page' : undefined}
          onClick={() => switchTab('installed')}
        >
          已安装 <span>{nativeAvailable && view ? view.local.length : '—'}</span>
        </button>
      </nav>
      {notice && (
        <div
          className={`n-alert ${notice.error ? 'n-error' : 'n-success'}`}
          role={notice.error ? 'alert' : 'status'}
        >
          {notice.error ? <CircleHelp size={16} /> : <CheckCircle2 size={16} />}
          <span>{notice.text}</span>
          <button onClick={() => setNotice(undefined)} aria-label="关闭提示">
            ×
          </button>
        </div>
      )}
      {tab === 'catalog' && catalogError && (
        <div className="n-offline-note" role="status">
          <CircleHelp size={16} />
          <span>
            {view?.source === 'cached'
              ? '广场同步失败，正在展示缓存目录。'
              : 'MCP 广场暂未连接，仍可查看本机已安装的 MCP。'}{' '}
            {catalogError}
          </span>
          <button disabled={catalogBusy} onClick={() => void refreshCatalog()}>
            重试
          </button>
        </div>
      )}
      {(tab === 'catalog' || nativeAvailable) && (
        <div className="n-catalog-toolbar n-mcp-toolbar">
          <label className="n-search">
            <Search size={16} />
            <input
              value={query}
              placeholder={tab === 'catalog' ? '搜索 MCP' : `搜索 ${agentName} 已安装的 MCP`}
              aria-label="搜索 MCP"
              onChange={(e) => setQuery(e.target.value)}
            />
          </label>
          <select
            className="n-category-select"
            aria-label="MCP 连接方式"
            value={transport}
            onChange={(e) => setTransport(e.target.value)}
          >
            <option value="">全部连接方式</option>
            <option value="stdio">stdio · 本地命令</option>
            <option value="http">HTTP · 远程地址</option>
          </select>
          <span aria-live="polite">
            {tab === 'catalog' ? catalogEntries.length : installedEntries.length} 个 MCP
          </span>
          <button
            className="n-button"
            disabled={!!busy || (tab === 'catalog' ? catalogBusy : !nativeAvailable)}
            onClick={() => (tab === 'catalog' ? void refreshCatalog() : void update('mcp-view'))}
          >
            <RefreshCw
              size={14}
              className={(tab === 'catalog' ? catalogBusy : busy === 'mcp-view') ? 'n-spin' : ''}
            />
            {tab === 'catalog'
              ? catalogBusy
                ? '正在同步…'
                : '同步广场'
              : busy === 'mcp-view'
                ? '正在检测…'
                : '重新检测'}
          </button>
        </div>
      )}
      {tab === 'installed' && nativeAvailable && (
        <div className="n-mcp-head">
          <div>
            <p>{workspaceNotes[workspace]}</p>
            {view?.configPath && <code>{view.configPath}</code>}
          </div>
        </div>
      )}

      {tab === 'catalog' ? (
        <>
          <div className="n-skill-list">
            {catalogEntries.map((server) => {
              const status = statusOf(server.id);
              const installed = status?.installed ?? false;
              return (
                <article
                  className="n-skill-row n-mcp-catalog-row"
                  key={server.id}
                  aria-label={server.name}
                >
                  <span className="n-skill-icon">
                    <Plug size={23} />
                  </span>
                  <div className="n-skill-info">
                    <h2>
                      {server.name}
                      <span className="n-pill">{transportLabel(server.transport.type)}</span>
                      <span className="n-version">
                        {status?.local ? '本机自定义' : `v${server.version}`}
                      </span>
                      {installed && <span className="n-pill n-ready">已安装 · {agentName}</span>}
                    </h2>
                    <p title={server.description}>{server.description}</p>
                    <div className="n-skill-meta">
                      <span>
                        {server.transport.type === 'stdio'
                          ? server.transport.command
                          : server.transport.url}
                      </span>
                      {server.capabilities.length > 0 && (
                        <span>{server.capabilities.join(' · ')}</span>
                      )}
                    </div>
                    {status && !status.tokenAvailable && (
                      <p className="n-mcp-warning">
                        未读取到本机 MCP 令牌，请先在对应软件中完成配置。
                      </p>
                    )}
                    {(server.steps.length > 0 || server.installCommand) && (
                      <details className="n-mcp-steps">
                        <summary>安装与使用说明</summary>
                        <ol>
                          {server.steps.map((step) => (
                            <li key={step}>{step}</li>
                          ))}
                        </ol>
                        {server.installCommand && <code>{server.installCommand}</code>}
                      </details>
                    )}
                  </div>
                  <div className="n-mcp-row-actions">
                    {!nativeAvailable ? (
                      <span className="n-config-note">请在桌面版接入 {agentName}</span>
                    ) : installed ? (
                      <button className="n-button" onClick={() => switchTab('installed')}>
                        <CheckCircle2 size={14} /> 管理已安装
                      </button>
                    ) : (
                      <button
                        className="n-button n-primary"
                        disabled={disabled || !status?.canInstall || !status.tokenAvailable}
                        onClick={() => void update('mcp-install', server.id)}
                      >
                        {busy === 'mcp-install' ? (
                          <LoaderCircle size={14} className="n-spin" />
                        ) : (
                          <Plug size={14} />
                        )}
                        接入 {agentName}
                      </button>
                    )}
                    {server.homepage && (
                      <a href={server.homepage} target="_blank" rel="noreferrer">
                        <Globe2 size={13} /> 项目主页
                      </a>
                    )}
                    {status?.local && (
                      <button
                        className="n-icon-button"
                        disabled={disabled}
                        onClick={() => void update('mcp-local-remove', server.id)}
                        aria-label={`删除本机条目 ${server.name}`}
                        title="删除本机条目"
                      >
                        <Trash2 size={14} />
                      </button>
                    )}
                  </div>
                </article>
              );
            })}
          </div>
          {!catalogEntries.length && (
            <div className="n-empty" role="status">
              {busy === 'read' || catalogBusy ? (
                <>
                  <LoaderCircle size={24} className="n-spin" />
                  <h2>正在读取 MCP 广场…</h2>
                </>
              ) : (
                <>
                  <Package size={28} />
                  <h2>
                    {filtered
                      ? '没有匹配的 MCP'
                      : view?.source === 'local' || catalogError
                        ? 'MCP 广场尚未同步'
                        : `暂无适用于 ${agentName} 的 MCP`}
                  </h2>
                  <p className="n-config-note">
                    {filtered
                      ? '试试其他名称或连接方式。'
                      : '同步广场获取可接入条目，也可以添加本机自定义 MCP。'}
                  </p>
                  {filtered ? (
                    <button
                      className="n-button"
                      onClick={() => {
                        setQuery('');
                        setTransport('');
                      }}
                    >
                      清除筛选
                    </button>
                  ) : (
                    <button
                      className="n-button"
                      disabled={catalogBusy}
                      onClick={() => void refreshCatalog()}
                    >
                      同步广场
                    </button>
                  )}
                </>
              )}
            </div>
          )}
          <section className="n-panel n-mcp-local" aria-label="本机自定义 MCP">
            <div className="n-panel-heading">
              <h2>本机自定义 MCP</h2>
              <span>保存后可在广场中接入</span>
            </div>
            <details className="n-mcp-local-add">
              <summary>添加一个本机 MCP</summary>
              <form
                className="n-mcp-local-form"
                onSubmit={(e) => {
                  e.preventDefault();
                  void saveLocal();
                }}
              >
                <label>
                  <span>标识</span>
                  <input
                    value={localForm.id}
                    placeholder="my-server"
                    required
                    onChange={(e) => setLocalForm({ ...localForm, id: e.target.value })}
                  />
                </label>
                <label>
                  <span>名称</span>
                  <input
                    value={localForm.name}
                    placeholder="我的 MCP"
                    onChange={(e) => setLocalForm({ ...localForm, name: e.target.value })}
                  />
                </label>
                <label>
                  <span>说明</span>
                  <input
                    value={localForm.description}
                    placeholder="本机启动的 MCP 服务器"
                    onChange={(e) => setLocalForm({ ...localForm, description: e.target.value })}
                  />
                </label>
                <label>
                  <span>方式</span>
                  <select
                    value={localForm.transportType}
                    onChange={(e) => setLocalForm({ ...localForm, transportType: e.target.value })}
                  >
                    <option value="stdio">stdio（本地命令）</option>
                    <option value="http">HTTP（远程地址）</option>
                  </select>
                </label>
                {localForm.transportType === 'stdio' ? (
                  <>
                    <label>
                      <span>启动命令</span>
                      <input
                        value={localForm.command}
                        placeholder="my-mcp-server"
                        required
                        onChange={(e) => setLocalForm({ ...localForm, command: e.target.value })}
                      />
                    </label>
                    <label>
                      <span>参数</span>
                      <input
                        value={localForm.args}
                        placeholder="--port 8080"
                        aria-describedby="mcp-args-help"
                        onChange={(e) => setLocalForm({ ...localForm, args: e.target.value })}
                      />
                      <small id="mcp-args-help">
                        空格分隔，含空格的参数用引号包裹；也支持 JSON 字符串数组。
                      </small>
                    </label>
                  </>
                ) : (
                  <label>
                    <span>地址</span>
                    <input
                      value={localForm.url}
                      placeholder="https://example.com/mcp"
                      required
                      onChange={(e) => setLocalForm({ ...localForm, url: e.target.value })}
                    />
                  </label>
                )}
                <div className="n-mcp-local-workspaces">
                  <span>适用软件</span>
                  {(['codex', 'claude', 'opencode'] as const).map((name) => (
                    <label key={name}>
                      <input
                        type="checkbox"
                        checked={localForm[name]}
                        onChange={(e) => setLocalForm({ ...localForm, [name]: e.target.checked })}
                      />
                      {workspaceNames[name]}
                    </label>
                  ))}
                </div>
                <button
                  type="submit"
                  className="n-button n-primary"
                  disabled={
                    disabled ||
                    !localForm.id.trim() ||
                    ![localForm.codex, localForm.claude, localForm.opencode].some(Boolean)
                  }
                >
                  {busy === 'mcp-local-save' ? (
                    <LoaderCircle size={14} className="n-spin" />
                  ) : (
                    <Plus size={14} />
                  )}
                  保存本机条目
                </button>
              </form>
            </details>
          </section>
        </>
      ) : !nativeAvailable ? (
        <div className="n-empty">
          <Package size={28} />
          <h2>请在桌面版查看 {agentName} 已安装的 MCP</h2>
          <p className="n-config-note">桌面版会读取当前软件的本机配置。</p>
        </div>
      ) : !view ? (
        <div className="n-empty" role="status">
          {busy ? (
            <>
              <LoaderCircle size={24} className="n-spin" />
              <h2>正在检测 {agentName} 的 MCP…</h2>
            </>
          ) : (
            <>
              <CircleHelp size={28} />
              <h2>暂时无法读取 {agentName} 的 MCP 配置</h2>
              <button className="n-button" onClick={() => void update('mcp-view')}>
                重新检测
              </button>
            </>
          )}
        </div>
      ) : (
        <>
          <div className="n-record-list n-mcp-installed-list">
            {installedEntries.map((entry) => {
              const server = view.catalog.servers.find((server) => server.id === entry.catalogId);
              const status = server ? statusOf(server.id) : undefined;
              return (
                <article
                  className="n-record n-mcp-installed-row"
                  key={entry.name}
                  aria-label={server?.name ?? entry.name}
                >
                  <span className="n-skill-icon">
                    <Plug size={23} />
                  </span>
                  <div className="n-mcp-installed-info">
                    <h2>
                      {server?.name ?? entry.name}
                      <span className="n-pill">{agentName}</span>
                      {workspace !== 'claude' && (
                        <span className={`n-pill ${entry.enabled ? 'n-ready' : 'n-warning'}`}>
                          {entry.enabled ? '已启用' : '已停用'}
                        </span>
                      )}
                    </h2>
                    <code>{entry.target || '配置未提供启动地址'}</code>
                    <p className="n-record-meta">
                      {entry.name} · {transportLabel(entry.transport)}
                      {workspace === 'claude' && ' · 全局接入'}
                    </p>
                    {status?.conflict && !entry.readOnlyReason && (
                      <p className="n-mcp-warning">
                        本机配置与广场条目不同，启停不会覆盖现有地址或参数。
                      </p>
                    )}
                    {entry.readOnlyReason && (
                      <p className="n-config-note">{entry.readOnlyReason}</p>
                    )}
                    {status && !status.tokenAvailable && (
                      <p className="n-mcp-warning">
                        未读取到本机 MCP 令牌，请先在对应软件中完成配置。
                      </p>
                    )}
                  </div>
                  <InstalledActions
                    entry={entry}
                    server={server}
                    status={status}
                    workspace={workspace}
                    disabled={disabled}
                    onAction={onAction}
                  />
                </article>
              );
            })}
          </div>
          {!installedEntries.length && (
            <div className="n-empty">
              <Package size={28} />
              <h2>{filtered ? '没有匹配的 MCP' : `${agentName} 暂无已安装的 MCP`}</h2>
              <p className="n-config-note">
                {filtered
                  ? '试试其他名称或连接方式。'
                  : `这里显示 ${agentName} 本机配置中检测到的 MCP。`}
              </p>
              {filtered ? (
                <button
                  className="n-button"
                  onClick={() => {
                    setQuery('');
                    setTransport('');
                  }}
                >
                  清除筛选
                </button>
              ) : (
                <button className="n-button n-primary" onClick={() => switchTab('catalog')}>
                  去 MCP 广场
                  <ArrowRight size={15} />
                </button>
              )}
            </div>
          )}
        </>
      )}
    </div>
  );
}
