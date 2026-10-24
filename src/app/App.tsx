import { lazy, Suspense, useCallback, useEffect, useRef, useState } from 'react';
import {
  ArrowDownToLine,
  ArrowRight,
  Activity,
  BarChart3,
  Check,
  CheckCircle2,
  ChevronDown,
  CircleHelp,
  Copy,
  Cpu,
  Download,
  FolderOpen,
  Globe2,
  Grid2X2,
  LoaderCircle,
  Moon,
  Package,
  Palette,
  Plug,
  RefreshCw,
  Search,
  Server,
  Settings2,
  SquareTerminal,
  Sun,
  Wifi,
  WifiOff,
  Zap,
} from 'lucide-react';
import ReactMarkdown from 'react-markdown';
import { getVersion } from '@tauri-apps/api/app';
import { Modal } from '../components/Primitives';
import { ConfigurationDialog } from './ConfigurationDialog';
import { CodexSettings } from './CodexSettings';
import { ClaudeSettings } from './ClaudeSettings';
import { OpenCodeSettings } from './OpenCodeSettings';
import { CodexDiagnostics } from './CodexDiagnostics';
import { ClaudeLogs } from './ClaudeLogs';
import { OpenCodeLogs } from './OpenCodeLogs';
import { LogWindow } from './LogWindow';
import { openLogs } from '../lib/api/diagnostics';
import { Integrations } from './Integrations';
import { McpManager } from './McpManager';
import { capability } from '../../shared/capabilities';
import thirdPartyNotices from '../../THIRD-PARTY-NOTICES.md?raw';
import type { ConfigurationTarget } from './ConfigurationDialog';
import { browserCatalog, nativeAvailable, nativeCall } from '../lib/api/native';
import type { NativePlan, NativeRecord, NativeStatus, ServerSkill } from '../lib/api/native';
import {
  checkUpdate,
  downloadUpdate,
  installUpdate,
  openExternal,
  rememberSkipped,
  skippedVersion,
  updateTargetUrl,
} from '../lib/api/update';
import type { UpdateStatus } from '../lib/api/update';
import { updateProgressText } from '../lib/update-progress';
import { CollectionCard } from '../features/skills/CollectionCard';
import { CollectionDialog } from '../features/skills/CollectionDialog';
import { SkillRow } from '../features/skills/SkillRow';
import { InstalledSkillRow } from '../features/skills/InstalledSkillRow';
import {
  collectionCatalog,
  collectionCategories,
  collectionInstallations,
  collectionInstalledEntries,
} from '../features/skills/collections';
import type { InstalledSkill, SkillCollection } from '../features/skills/collections';
import type { CollectionLocation } from '../features/skills/collection-install';
import { useCollectionRun } from '../features/skills/useCollectionRun';
import { applyTheme, loadTheme, syncNativeTheme } from '../lib/theme';
import type { ThemePreference } from '../lib/theme';
import { visibleInWorkspace } from '../lib/installations';
import { readCyapiRoutes } from '../lib/api/cyapi';
import {
  APP_NAME,
  APP_VERSION,
  COPYRIGHT_SHORT,
  COPYRIGHT_TEXT,
  DOWNLOAD_PAGE,
  OFFICIAL_SITE,
} from '../lib/app-info';

const UsageStatistics = lazy(() =>
  import('./UsageStatistics').then((module) => ({ default: module.UsageStatistics })),
);

type Page =
  | 'catalog'
  | 'installed'
  | 'codex'
  | 'mcp'
  | 'diagnostics'
  | 'usage'
  | 'environment'
  | 'settings'
  | 'integrations';
type Workspace = 'codex' | 'claude' | 'opencode';
const agents = [
  { id: 'codex', name: 'Codex', hint: 'OpenAI' },
  { id: 'claude', name: 'Claude Code', hint: 'Anthropic' },
  { id: 'opencode', name: 'OpenCode', hint: '开源' },
] as const;
const targetLabels = {
  codex: 'Codex',
  claude: 'Claude Code',
  opencode: 'OpenCode',
} as const;
const workspaceFromStorage = (): Workspace => {
  const saved = localStorage.getItem('cyapi.workspace');
  return saved === 'claude' || saved === 'opencode' ? saved : 'codex';
};
const pages = {
  catalog: capability('skills').title,
  installed: capability('skills').title,
  codex: '模型接入',
  mcp: capability('mcp').title,
  diagnostics: '日志',
  usage: '用量统计',
  environment: '运行环境',
  settings: '设置',
  integrations: capability('integrations').title,
};
const sourceLabels = { codex: 'Codex 自带', system: '系统环境', managed: '工具箱共享' };
const errorText = (e: unknown) => (e instanceof Error ? e.message : String(e));
const normalizePath = (path: string) =>
  path.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();

export default function App() {
  const [logsOpen, setLogsOpen] = useState(false);
  const [logOperation, setLogOperation] = useState<string>();
  const [theme, setThemeState] = useState<ThemePreference>(() => {
    const stored = loadTheme();
    applyTheme(stored);
    return stored;
  });
  const setTheme = useCallback((preference: ThemePreference) => {
    applyTheme(preference);
    setThemeState(preference);
  }, []);
  const themeLabel = theme === 'system' ? '跟随系统' : theme === 'light' ? '浅色' : '深色';
  // The native window follows the same appearance as the page.
  useEffect(() => syncNativeTheme(theme), [theme]);
  const lastOperation = useRef<string | undefined>(undefined);
  useEffect(() => {
    const show = (event: Event) => {
      setLogOperation((event as CustomEvent<{ operationId?: string }>).detail?.operationId);
      setLogsOpen(true);
    };
    const track = (event: Event) => {
      lastOperation.current = (event as CustomEvent<{ operationId: string }>).detail.operationId;
    };
    window.addEventListener('toolbox-open-logs', show);
    window.addEventListener('toolbox-operation', track);
    window.addEventListener('toolbox-operation-failed', show);
    return () => {
      window.removeEventListener('toolbox-open-logs', show);
      window.removeEventListener('toolbox-operation', track);
      window.removeEventListener('toolbox-operation-failed', show);
    };
  }, []);
  const [page, setPage] = useState<Page>('codex');
  const [mcpFocus, setMcpFocus] = useState<{ query: string; tab: 'catalog' | 'installed' }>();
  const [workspace, setWorkspace] = useState<Workspace>(workspaceFromStorage);
  const [workspaceOpen, setWorkspaceOpen] = useState(false);
  const workspaceMenu = useRef<HTMLDivElement>(null);
  const [status, setStatus] = useState<NativeStatus>();
  const [query, setQuery] = useState('');
  const [category, setCategory] = useState('');
  const [recommendedOnly, setRecommendedOnly] = useState(false);
  const [busy, setBusy] = useState('');
  const [notice, setNotice] = useState<{ text: string; error?: boolean }>();
  const [selected, setSelected] = useState<ServerSkill>();
  const [selectedCollection, setSelectedCollection] = useState<{
    collection: SkillCollection;
    location: CollectionLocation;
    installedEntries?: InstalledSkill[];
  }>();
  const [target, setTarget] = useState<'codex' | 'claude' | 'opencode'>('codex');
  const [scope, setScope] = useState<'global' | 'project'>('global');
  const [projectPath, setProjectPath] = useState('');
  const [plan, setPlan] = useState<NativePlan>();
  const [planError, setPlanError] = useState('');
  const [completed, setCompleted] = useState<NativeRecord>();
  const [remove, setRemove] = useState<NativeRecord>();
  const [help, setHelp] = useState(false);
  const [telemetryEnabled, setTelemetryEnabled] = useState<boolean>();
  const [telemetryError, setTelemetryError] = useState('');
  const [telemetryBusy, setTelemetryBusy] = useState(false);
  const [appVersion, setAppVersion] = useState(APP_VERSION);
  const [update, setUpdate] = useState<UpdateStatus>();
  const [updateHidden, setUpdateHidden] = useState(false);
  const [updateStep, setUpdateStep] = useState('');
  const [updateError, setUpdateError] = useState('');
  const [updateChecking, setUpdateChecking] = useState(false);
  const [updateCheckError, setUpdateCheckError] = useState('');
  const [updateCheckedAt, setUpdateCheckedAt] = useState<number>();
  const updateRequest = useRef(0);
  const updateInstalling = useRef(false);
  const [configuration, setConfiguration] = useState<ConfigurationTarget>();
  const main = useRef<HTMLElement>(null);
  const planVersion = useRef(0);
  const modalOpen = Boolean(selected || selectedCollection || remove || help || configuration);
  const skillPage = page === 'catalog' || page === 'installed';
  // The suite task lives outside the dialog, so closing the window keeps the
  // installation running and the user can still stop it.
  const collectionRun = useCollectionRun({
    onBusy: setBusy,
    onStatus: (next, target, installedEntries) => {
      setStatus(next);
      setWorkspace(target);
      setSelectedCollection((current) => (current ? { ...current, installedEntries } : current));
    },
    installedEntriesFor: (next, collectionId, target, scope, skillPath) => {
      const group = next.skillGroups?.find((item) => item.id === collectionId);
      return group ? collectionInstalledEntries(next, group, target, scope, skillPath) : [];
    },
  });

  const checkForUpdates = useCallback(async (fromSettings = false) => {
    if (updateInstalling.current) return;
    const request = ++updateRequest.current;
    setUpdateChecking(true);
    setUpdateCheckError('');
    setUpdateError('');
    try {
      const next = await checkUpdate();
      // A slower startup check must not overwrite a later settings check.
      if (request !== updateRequest.current) return;
      setUpdate(next);
      setUpdateHidden(!fromSettings && !next.mandatory && next.latest === skippedVersion());
      setUpdateCheckedAt(next.source === 'cached' ? undefined : Date.now());
      if (next.source === 'cached') {
        setUpdateCheckError(next.checkError || '无法连接更新服务');
      }
    } catch (e) {
      if (request !== updateRequest.current) return;
      setUpdate(undefined);
      setUpdateCheckedAt(undefined);
      setUpdateCheckError(errorText(e));
    } finally {
      if (request === updateRequest.current) setUpdateChecking(false);
    }
  }, []);

  useEffect(() => {
    main.current?.scrollTo({ top: 0 });
  }, [page, workspace]);
  useEffect(() => {
    localStorage.setItem('cyapi.workspace', workspace);
    setTarget(workspace);
  }, [workspace]);
  useEffect(() => {
    if (!workspaceOpen) return;
    const close = (event: PointerEvent) => {
      if (!workspaceMenu.current?.contains(event.target as Node)) setWorkspaceOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key === 'Escape') setWorkspaceOpen(false);
    };
    document.addEventListener('pointerdown', close);
    document.addEventListener('keydown', escape);
    return () => {
      document.removeEventListener('pointerdown', close);
      document.removeEventListener('keydown', escape);
    };
  }, [workspaceOpen]);
  useEffect(() => {
    let active = true;
    async function start() {
      // Fetch the route manifest once per application session. Settings pages and
      // workspace switches reuse the cached snapshot until the user refreshes it.
      void readCyapiRoutes().catch(() => {});
      setBusy('正在加载工作空间');
      try {
        if (nativeAvailable) {
          const cached = await nativeCall<NativeStatus>('status');
          if (active) {
            setStatus(cached);
          }
          const next = await nativeCall<NativeStatus>('catalog-read');
          if (active) setStatus(next);
        } else {
          const next = await browserCatalog();
          if (active) setStatus(next);
        }
      } catch (e) {
        if (active) setNotice({ text: errorText(e), error: true });
      } finally {
        if (active) setBusy('');
      }
    }
    void start();
    return () => {
      active = false;
    };
  }, []);
  useEffect(() => {
    if (!nativeAvailable) return;
    getVersion()
      .then(setAppVersion)
      .catch(() => {});
  }, []);
  useEffect(() => {
    void checkForUpdates();
    return () => {
      updateRequest.current += 1;
    };
  }, [checkForUpdates]);
  useEffect(() => {
    // Opening settings always checks again, including previously skipped releases.
    if (page === 'settings') {
      void checkForUpdates(true);
      if (nativeAvailable)
        void nativeCall<{ enabled: boolean }>('telemetry-read')
          .then((value) => {
            setTelemetryEnabled(value.enabled);
            setTelemetryError('');
          })
          .catch(() => setTelemetryError('无法读取统计设置，请稍后重试。'));
    }
  }, [page, checkForUpdates]);
  useEffect(() => {
    const current = ++planVersion.current;
    setPlan(undefined);
    setPlanError('');
    if (!selected || selected.distribution === 'source-only' || !nativeAvailable) return;
    const timer = setTimeout(() => {
      nativeCall<NativePlan>('preview', { skillId: selected.id, target, scope, projectPath })
        .then((next) => {
          if (current === planVersion.current) setPlan(next);
        })
        .catch((e) => {
          if (current === planVersion.current) setPlanError(errorText(e));
        });
    }, 200);
    return () => clearTimeout(timer);
  }, [selected, target, scope, projectPath, status]);

  async function refresh(action = 'refresh') {
    setBusy(action === 'rescan' ? '正在检测可用环境' : '正在连接服务端');
    setNotice(undefined);
    try {
      setStatus(nativeAvailable ? await nativeCall<NativeStatus>(action) : await browserCatalog());
    } catch (e) {
      setNotice({ text: errorText(e), error: true });
    } finally {
      setBusy('');
    }
  }
  async function download(skill: ServerSkill) {
    setBusy('正在下载离线文件');
    setNotice(undefined);
    try {
      setStatus(await nativeCall<NativeStatus>('download', { skillId: skill.id }));
      setNotice({ text: '下载完成，可以离线安装。' });
    } catch (e) {
      setNotice({ text: errorText(e), error: true });
    } finally {
      setBusy('');
    }
  }
  async function install() {
    if (
      !selected ||
      !plan ||
      plan.conflict ||
      plan.runtimes.some((runtime) => !runtime.reuse && !runtime.fallback)
    ) {
      return;
    }
    setBusy('正在安装 Skill');
    setNotice(undefined);
    try {
      const record = await nativeCall<NativeRecord>('install', {
        skillId: selected.id,
        target,
        scope,
        projectPath,
      });
      setCompleted(record);
      setWorkspace(record.target);
      setStatus(await nativeCall<NativeStatus>('status'));
    } catch (e) {
      setNotice({ text: errorText(e), error: true });
    } finally {
      setBusy('');
    }
  }
  async function updateSkill(skillId: string) {
    setBusy('正在更新 Skill');
    setNotice(undefined);
    try {
      const record = await nativeCall<NativeRecord>('update-skill', {
        skillId,
        target: workspace,
        scope: 'global',
        projectPath: '',
      });
      setStatus(await nativeCall<NativeStatus>('status'));
      setNotice({ text: `已更新到 ${record.version}，旧版本保留在备份目录。` });
    } catch (e) {
      setNotice({ text: errorText(e), error: true });
    } finally {
      setBusy('');
    }
  }
  async function uninstall() {
    if (!remove) return;
    setBusy('正在将 Skill 移至备份');
    try {
      const result = await nativeCall<{ backupPath: string }>('remove', { recordId: remove.id });
      setRemove(undefined);
      setStatus(await nativeCall<NativeStatus>('status'));
      setNotice({ text: `已卸载，原文件保留在：${result.backupPath}` });
    } catch (e) {
      setNotice({ text: errorText(e), error: true });
    } finally {
      setBusy('');
    }
  }
  async function copy(text: string) {
    try {
      await navigator.clipboard.writeText(text);
      setNotice({ text: '已复制' });
    } catch {
      setNotice({ text: '复制失败，请手动复制。', error: true });
    }
  }
  function select(skill: ServerSkill) {
    setCompleted(undefined);
    setTarget(workspace);
    setSelected(skill);
    setNotice(undefined);
  }
  function dismissUpdate() {
    setUpdateHidden(true);
  }
  function skipUpdate() {
    if (update) rememberSkipped(update.latest);
    setUpdateHidden(true);
  }
  async function openUpdatePage() {
    if (!update) return;
    setUpdateError('');
    try {
      await openExternal(updateTargetUrl(update));
    } catch (e) {
      setUpdateError(errorText(e));
    }
  }
  /** Open a project page in the system browser instead of the embedded webview. */
  async function openSite(url: string) {
    setNotice(undefined);
    try {
      await openExternal(url);
    } catch (e) {
      setNotice({ text: errorText(e), error: true });
    }
  }
  async function installUpdateNow() {
    if (!update || updateChecking || updateInstalling.current) return;
    updateInstalling.current = true;
    setUpdateError('');
    setUpdateStep('正在下载更新安装包');
    try {
      await downloadUpdate((progress) => setUpdateStep(updateProgressText(progress)));
      setUpdateStep('正在校验并启动安装程序');
      await installUpdate();
      setUpdateStep('安装程序已启动，应用将自动关闭');
    } catch (e) {
      updateInstalling.current = false;
      setUpdateStep('');
      setUpdateError(errorText(e));
    }
  }
  const workspaceInstalled = (status?.installed ?? []).filter((entry) =>
    visibleInWorkspace(entry, workspace),
  );
  const workspacePaths = new Set(
    workspaceInstalled.map(({ record }) => normalizePath(record.path)),
  );
  const catalogSkills = (status?.skills ?? []).map((entry) => {
    const installedPaths = (entry.installedPaths ?? []).filter((path) =>
      workspacePaths.has(normalizePath(path)),
    );
    return { ...entry, installed: installedPaths.length > 0, installedPaths };
  });
  const skillGroups = status?.skillGroups ?? [];
  const categories = collectionCategories(catalogSkills, skillGroups);
  const skills = collectionCatalog(catalogSkills, skillGroups, query, category, recommendedOnly);
  const catalogCount = collectionCatalog(catalogSkills, skillGroups).length;
  const installedItems = collectionInstallations(workspaceInstalled, catalogSkills, skillGroups);
  const manageCollection = (
    collection: SkillCollection,
    location: CollectionLocation = {
      target: workspace,
      scope: 'global',
      projectPath: '',
    },
    installedEntries?: InstalledSkill[],
  ) => {
    setNotice(undefined);
    setSelectedCollection({ collection, location, installedEntries });
  };
  const renderInstalledSkill = (entry: InstalledSkill) => (
    <InstalledSkillRow
      key={entry.record.id}
      entry={entry}
      workspace={workspace}
      busy={!!busy}
      configurable={Boolean(
        entry.record.configuration ||
        status?.skills.some(
          ({ skill }) =>
            skill.id === entry.record.skillId &&
            skill.revision === entry.record.revision &&
            skill.configuration,
        ),
      )}
      onCopy={(path) => void copy(path)}
      onConfigure={(record) => setConfiguration({ title: record.title, recordId: record.id })}
      onRemove={(record) => {
        setNotice(undefined);
        setRemove(record);
      }}
    />
  );
  const selectedReady =
    status?.skills.find((entry) => entry.skill.id === selected?.id)?.offlineReady ?? false;
  const alert = notice && (
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
  );
  const progress = busy && (
    <div className="n-progress" role="status">
      <LoaderCircle size={15} className="n-spin" />
      <span className="n-progress-text">{busy}</span>
      {collectionRun.run.running && (
        <button
          className="n-button n-danger n-progress-action"
          disabled={collectionRun.run.cancelling}
          onClick={collectionRun.cancel}
        >
          {collectionRun.run.cancelling ? '正在取消…' : '取消'}
        </button>
      )}
    </div>
  );
  const updateBanner = update?.hasUpdate &&
    (page === 'settings' || !updateHidden) &&
    !(page === 'settings' && updateChecking) && (
      <section
        className={`n-update ${update.mandatory ? 'n-update-required' : ''}`}
        role="status"
        aria-label="客户端更新"
      >
        <div className="n-update-head">
          <ArrowDownToLine size={16} />
          <strong>发现新版本 v{update.latest}</strong>
          <span>当前 v{update.current || appVersion}</span>
          {update.mandatory && <span className="n-update-tag">建议立即更新</span>}
        </div>
        {update.notes.length > 0 && (
          <ul className="n-update-notes">
            {update.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        )}
        {updateStep && (
          <div className="n-progress" role="status">
            <LoaderCircle size={15} className="n-spin" />
            {updateStep}
          </div>
        )}
        {updateError && <p className="n-error-text">{updateError}</p>}
        <div className="n-update-actions">
          {update.canSelfUpdate && (
            <button
              className="n-button n-primary"
              disabled={!!updateStep || updateChecking}
              onClick={() => void installUpdateNow()}
            >
              <ArrowDownToLine size={14} />
              立即更新
            </button>
          )}
          <button className="n-button" onClick={() => void openUpdatePage()}>
            <Download size={14} />
            {update.canSelfUpdate ? '从官网下载' : '下载新版本'}
          </button>
          {!update.mandatory && page !== 'settings' && (
            <button className="n-button" disabled={!!updateStep} onClick={skipUpdate}>
              跳过此版本
            </button>
          )}
          {!update.mandatory && page !== 'settings' && (
            <button className="n-update-link" disabled={!!updateStep} onClick={dismissUpdate}>
              稍后提醒
            </button>
          )}
        </div>
      </section>
    );

  const agent = agents.find((item) => item.id === workspace) ?? agents[0];
  const agentPage =
    page === 'codex' || page === 'mcp' || page === 'diagnostics' || page === 'usage';

  return (
    <div className="app-shell">
      <aside className="sidebar" inert={modalOpen}>
        <button className="brand" onClick={() => setPage('codex')} aria-label="CYAPI Toolbox 首页">
          <span className="brand-mark">
            <Zap size={20} fill="currentColor" />
          </span>
          <span>
            <strong>CYAPI</strong>
            <small>TOOLBOX</small>
          </span>
        </button>
        <div className="n-workspace" ref={workspaceMenu}>
          <span className="n-workspace-label">工作区</span>
          <button
            className="n-workspace-toggle"
            aria-haspopup="listbox"
            aria-expanded={workspaceOpen}
            onClick={() => setWorkspaceOpen(!workspaceOpen)}
          >
            <span className={`n-workspace-dot ${workspace}`} />
            <span>{agent.name}</span>
            <ChevronDown size={14} className={workspaceOpen ? 'open' : ''} />
          </button>
          {workspaceOpen && (
            <div className="n-workspace-menu" role="listbox" aria-label="选择 Agent 工作区">
              {agents.map((item) => (
                <button
                  key={item.id}
                  role="option"
                  aria-selected={workspace === item.id}
                  className={workspace === item.id ? 'active' : ''}
                  onClick={() => {
                    setWorkspace(item.id);
                    setWorkspaceOpen(false);
                  }}
                >
                  <span className={`n-workspace-dot ${item.id}`} />
                  <span>
                    <strong>{item.name}</strong>
                    <small>{item.hint}</small>
                  </span>
                  {workspace === item.id && <Check size={14} />}
                </button>
              ))}
            </div>
          )}
        </div>
        <nav className="primary-nav" aria-label="主导航">
          {(
            [
              ['codex', SquareTerminal, '模型接入'],
              ['integrations', Plug, capability('integrations').title],
              ['catalog', Grid2X2, 'Skill 管理'],
              ['mcp', Server, 'MCP 管理'],
              ['environment', Cpu, '运行环境'],
              ['usage', BarChart3, '用量统计'],
              ['diagnostics', Activity, '日志'],
            ] as const
          ).map(([id, Icon, label]) => {
            const active = id === 'catalog' ? skillPage : page === id;
            return (
              <button
                key={id}
                className={active ? 'active' : ''}
                onClick={() => {
                  setMcpFocus(undefined);
                  setPage(id);
                }}
                aria-current={active ? 'page' : undefined}
                title={label}
              >
                <Icon size={18} />
                <span>{label}</span>
              </button>
            );
          })}
        </nav>
        <div className="n-sidebar-bottom">
          <div className="n-sidebar-tools" role="group" aria-label="辅助工具">
            <button
              onClick={() => (logsOpen ? setLogsOpen(false) : openLogs())}
              className={logsOpen ? 'active' : ''}
              title={logsOpen ? '关闭工具箱日志' : '工具箱日志'}
              aria-label={logsOpen ? '关闭工具箱日志' : '工具箱日志'}
              aria-expanded={logsOpen}
              data-logs-trigger
            >
              <SquareTerminal size={17} />
            </button>
            <button
              onClick={() => setPage('settings')}
              className={page === 'settings' ? 'active' : ''}
              aria-current={page === 'settings' ? 'page' : undefined}
              title="设置"
              aria-label="设置"
            >
              <Settings2 size={17} />
            </button>
            <button onClick={() => setHelp(true)} title="使用帮助" aria-label="使用帮助">
              <CircleHelp size={17} />
            </button>
          </div>
          <small className="n-sidebar-meta">
            <span>v{appVersion}</span>
            {update?.hasUpdate && !updateHidden && (
              <span className="n-sidebar-update">有新版本</span>
            )}
            <span className="n-sidebar-copyright">{COPYRIGHT_SHORT}</span>
          </small>
        </div>
      </aside>
      <div className="n-shell" inert={modalOpen}>
        <header className="n-page-heading">
          <h1>{agentPage ? `${agent.name} · ${pages[page]}` : pages[page]}</h1>
          {page !== 'codex' &&
            page !== 'mcp' &&
            page !== 'integrations' &&
            page !== 'usage' &&
            page !== 'diagnostics' && (
              <div className="n-heading-actions">
                <span
                  className={`n-connection ${status?.online ? 'online' : ''}`}
                  title={status?.message}
                >
                  {status?.online ? <Wifi size={14} /> : <WifiOff size={14} />}
                  {status?.online ? '服务已连接' : status?.skills.length ? '离线目录' : '未连接'}
                </span>
                <button
                  className="n-button"
                  disabled={!!busy}
                  onClick={() => refresh(page === 'environment' ? 'rescan' : 'refresh')}
                >
                  <RefreshCw size={14} className={busy ? 'n-spin' : ''} />
                  {page === 'environment' ? '重新检测' : '同步目录'}
                </button>
              </div>
            )}
        </header>
        <main className="n-main" ref={main}>
          <div className="n-content">
            {!nativeAvailable && <p className="n-browser-note">网页预览 · 本机操作请使用桌面版</p>}
            {page !== 'settings' && updateBanner}
            {skillPage && (
              <nav className="n-skill-tabs" aria-label="Skill 管理视图">
                <button
                  className={page === 'catalog' ? 'active' : ''}
                  aria-current={page === 'catalog' ? 'page' : undefined}
                  onClick={() => setPage('catalog')}
                >
                  Skill 广场
                </button>
                <button
                  className={page === 'installed' ? 'active' : ''}
                  aria-current={page === 'installed' ? 'page' : undefined}
                  onClick={() => setPage('installed')}
                >
                  已安装 <span>{workspaceInstalled.length}</span>
                </button>
              </nav>
            )}
            {!modalOpen && (
              <>
                {page !== 'codex' &&
                  page !== 'mcp' &&
                  page !== 'usage' &&
                  page !== 'integrations' &&
                  alert}
                {progress}
              </>
            )}

            {page === 'codex' &&
              (workspace === 'claude' ? (
                <ClaudeSettings />
              ) : workspace === 'opencode' ? (
                <OpenCodeSettings />
              ) : (
                <CodexSettings />
              ))}
            {page === 'usage' && (
              <Suspense fallback={<p role="status">正在加载用量统计…</p>}>
                <UsageStatistics key={workspace} workspace={workspace} />
              </Suspense>
            )}
            {page === 'diagnostics' &&
              (workspace === 'claude' ? (
                <ClaudeLogs />
              ) : workspace === 'opencode' ? (
                <OpenCodeLogs />
              ) : (
                <CodexDiagnostics />
              ))}
            {page === 'mcp' && <McpManager workspace={workspace} focus={mcpFocus} />}
            {page === 'integrations' && (
              <Integrations
                workspace={workspace}
                onManageMcp={(id, installed) => {
                  setMcpFocus({ query: id, tab: installed ? 'installed' : 'catalog' });
                  setPage('mcp');
                }}
                onManageSkill={(id) => {
                  setQuery(id);
                  setCategory('');
                  setRecommendedOnly(false);
                  setPage('catalog');
                }}
              />
            )}
            {page === 'catalog' && (
              <>
                {status?.message && (
                  <div className="n-offline-note" title={status.message}>
                    <WifiOff size={15} />
                    {status.skills.length
                      ? '服务未连接，使用缓存目录。'
                      : '服务暂未连接，请稍后重试。'}
                    <button disabled={!!busy} onClick={() => refresh()}>
                      重新连接
                    </button>
                  </div>
                )}
                <div className="n-catalog-toolbar">
                  <label className="n-search">
                    <Search size={16} />
                    <input
                      placeholder="搜索 Skill"
                      aria-label="搜索 Skill"
                      value={query}
                      onChange={(e) => setQuery(e.target.value)}
                    />
                  </label>
                  <select
                    className="n-category-select"
                    aria-label="Skill 分类"
                    value={category}
                    onChange={(e) => setCategory(e.target.value)}
                  >
                    <option value="">全部分类（{catalogCount}）</option>
                    {categories.map(([name, count]) => (
                      <option key={name} value={name}>
                        {name}（{count}）
                      </option>
                    ))}
                  </select>
                  <label className="n-catalog-recommended">
                    <input
                      type="checkbox"
                      checked={recommendedOnly}
                      onChange={(e) => setRecommendedOnly(e.target.checked)}
                    />
                    只看推荐
                  </label>
                  <span aria-live="polite">{skills.length} 个 Skill / 套件</span>
                </div>
                <div className="n-skill-list">
                  {skills.map((item) =>
                    item.kind === 'skill' ? (
                      <SkillRow
                        key={item.entry.skill.id}
                        entry={item.entry}
                        busy={!!busy}
                        onSelect={select}
                        onUpdate={(id) => void updateSkill(id)}
                        onConfigure={(skill) =>
                          setConfiguration({
                            title: skill.title,
                            skillId: skill.id,
                            preview: skill.configuration,
                          })
                        }
                      />
                    ) : (
                      <CollectionCard
                        key={item.collection.id}
                        collection={item.collection}
                        count={item.matches.length}
                        searching={!!query.trim() && item.matches.length < item.entries.length}
                        busy={!!busy && !collectionRun.run.running}
                        onManage={() => {
                          const existing = installedItems.find(
                            (installed) =>
                              installed.kind === 'collection' &&
                              installed.collection.id === item.collection.id &&
                              installed.entries[0].record.target === workspace &&
                              installed.entries[0].record.scope === 'global',
                          );
                          manageCollection(
                            item.collection,
                            { target: workspace, scope: 'global', projectPath: '' },
                            existing?.kind === 'collection' ? existing.entries : undefined,
                          );
                        }}
                        meta={
                          <>
                            <span>{item.entries.length} 个模块</span>
                            <span>
                              已安装 {item.entries.filter((entry) => entry.installed).length}/
                              {item.entries.length}
                            </span>
                            {item.entries.some((entry) => entry.updateAvailable) && (
                              <span className="n-pill n-warning">可更新</span>
                            )}
                            {item.matches.length < item.entries.length && (
                              <span>匹配 {item.matches.length} 项</span>
                            )}
                          </>
                        }
                      >
                        {item.matches.map((entry) => (
                          <SkillRow
                            key={entry.skill.id}
                            entry={entry}
                            busy={!!busy}
                            onSelect={select}
                            onUpdate={(id) => void updateSkill(id)}
                            onConfigure={(skill) =>
                              setConfiguration({
                                title: skill.title,
                                skillId: skill.id,
                                preview: skill.configuration,
                              })
                            }
                          />
                        ))}
                      </CollectionCard>
                    ),
                  )}
                </div>
                {!skills.length && (
                  <div className="n-empty">
                    <Package size={28} />
                    <h2>{query || category ? '没有匹配的 Skill' : '暂无 Skill'}</h2>
                    {query || category ? (
                      <button
                        className="n-button"
                        onClick={() => {
                          setQuery('');
                          setCategory('');
                        }}
                      >
                        清除筛选
                      </button>
                    ) : (
                      <button className="n-button" disabled={!!busy} onClick={() => refresh()}>
                        重新同步
                      </button>
                    )}
                  </div>
                )}
              </>
            )}

            {page === 'installed' && (
              <>
                <div className="n-record-list">
                  {installedItems.map((item) =>
                    item.kind === 'skill' ? (
                      renderInstalledSkill(item.entry)
                    ) : (
                      <CollectionCard
                        key={item.key}
                        collection={item.collection}
                        count={item.entries.length}
                        description={item.root}
                        busy={!!busy && !collectionRun.run.running}
                        meta={
                          <>
                            <span>
                              {targetLabels[item.entries[0].record.target]} ·{' '}
                              {item.entries[0].record.scope === 'global' ? '全局' : '项目'}
                            </span>
                            <span>
                              已安装 {item.entries.filter((entry) => entry.exists).length} 个模块
                            </span>
                            {item.entries[0].record.target !== workspace && (
                              <span>在原工作区管理</span>
                            )}
                          </>
                        }
                        onManage={
                          item.entries[0].record.target === workspace
                            ? () =>
                                manageCollection(
                                  item.collection,
                                  {
                                    target: item.entries[0].record.target,
                                    scope: item.entries[0].record.scope,
                                    projectPath:
                                      item.entries[0].record.scope === 'project'
                                        ? item.root.replace(
                                            /\/\.(agents|claude|opencode)\/skills$/i,
                                            '',
                                          )
                                        : '',
                                  },
                                  item.entries,
                                )
                            : undefined
                        }
                      >
                        {item.entries.map(renderInstalledSkill)}
                      </CollectionCard>
                    ),
                  )}
                </div>
                {!workspaceInstalled.length && (
                  <div className="n-empty">
                    <Package size={28} />
                    <h2>{targetLabels[workspace]} 暂无安装记录</h2>
                    <p className="n-config-note">
                      当前工作区显示 {targetLabels[workspace]} 可读取的 Skill。
                    </p>
                    <button className="n-button n-primary" onClick={() => setPage('catalog')}>
                      去安装 Skill <ArrowRight size={15} />
                    </button>
                  </div>
                )}
              </>
            )}

            {page === 'environment' &&
              (['node', 'python'] as const).map((kind) => (
                <section className="n-panel" key={kind}>
                  <div className="n-panel-heading">
                    <h2>{kind === 'node' ? 'Node.js' : 'Python'}</h2>
                    {kind === 'python' && <span>仅检测</span>}
                  </div>
                  {status?.runtimes
                    .filter((r) => r.kind === kind)
                    .map((runtime) => (
                      <div className="n-runtime" key={runtime.path}>
                        <div>
                          <strong className="mono">v{runtime.version}</strong>
                          <span className="n-pill">{sourceLabels[runtime.source]}</span>
                          <code>{runtime.path}</code>
                        </div>
                        <button
                          className="n-icon-button"
                          onClick={() => copy(runtime.path)}
                          aria-label={`复制 ${kind} ${runtime.version} 路径`}
                          title="复制路径"
                        >
                          <Copy size={16} />
                        </button>
                      </div>
                    ))}
                  {!status?.runtimes.some((r) => r.kind === kind) && (
                    <p className="n-panel-empty">
                      {nativeAvailable ? '未检测到可用环境' : '请在桌面版检测'}
                    </p>
                  )}
                </section>
              ))}

            {page === 'settings' && (
              <>
                <section className="n-panel" aria-label="使用统计">
                  <div className="n-panel-heading">
                    <h2>使用统计</h2>
                  </div>
                  <p className="n-config-note">
                    发送公开 Skill 的下载校验、安装与更新结果，以及公开 MCP
                    的配置接入结果，用于改进服务。统计包含条目版本、平台、目标软件、耗时和随机设备标识；不包含
                    API Key、命令、文件路径或模型请求。
                  </p>
                  <button
                    className="n-button"
                    disabled={!nativeAvailable || telemetryEnabled === undefined || telemetryBusy}
                    onClick={async () => {
                      setTelemetryBusy(true);
                      setTelemetryError('');
                      try {
                        const value = await nativeCall<{ enabled: boolean }>(
                          telemetryEnabled ? 'telemetry-disable' : 'telemetry-enable',
                        );
                        setTelemetryEnabled(value.enabled);
                      } catch {
                        setTelemetryError('无法保存统计设置，请稍后重试。');
                      } finally {
                        setTelemetryBusy(false);
                      }
                    }}
                  >
                    {telemetryBusy
                      ? '正在保存…'
                      : telemetryEnabled === undefined
                        ? '正在读取…'
                        : telemetryEnabled
                          ? '关闭使用统计'
                          : '开启使用统计'}
                  </button>
                  <p className="n-config-note">关闭后停止新增和发送回执，并清除本地待发送队列。</p>
                  {telemetryError && (
                    <p className="n-error-text" role="alert">
                      {telemetryError}
                    </p>
                  )}
                </section>
                <section
                  className="n-panel n-update-panel"
                  aria-label="软件更新"
                  aria-busy={updateChecking}
                >
                  <div className="n-panel-heading">
                    <h2>检查更新</h2>
                    <button
                      className="n-button"
                      disabled={updateChecking || !!updateStep}
                      onClick={() => void checkForUpdates(true)}
                    >
                      <RefreshCw size={14} className={updateChecking ? 'n-spin' : ''} />
                      {updateChecking ? '正在检查…' : '检查更新'}
                    </button>
                  </div>
                  <p className="n-config-note">每次进入设置时自动检查，也可手动检查。</p>
                  <div className="n-update-versions">
                    <span>
                      当前版本 <strong>v{appVersion}</strong>
                    </span>
                    {update && !updateChecking && (
                      <span>
                        {update.source === 'cached' ? '上次获取版本' : '最新发布版本'}{' '}
                        <strong>v{update.latest}</strong>
                      </span>
                    )}
                  </div>
                  <div
                    className={`n-alert ${updateCheckError && !updateChecking ? 'n-error' : 'n-success'}`}
                    role={updateCheckError && !updateChecking ? 'alert' : 'status'}
                  >
                    {updateChecking ? (
                      <LoaderCircle size={16} className="n-spin" />
                    ) : updateCheckError ? (
                      <WifiOff size={16} />
                    ) : (
                      <CheckCircle2 size={16} />
                    )}
                    <span>
                      {updateChecking
                        ? '正在获取最新版本信息…'
                        : updateCheckError
                          ? update?.source === 'cached'
                            ? '暂时无法获取最新版本，以下为上次的更新信息。'
                            : '检查更新失败，请稍后重试。'
                          : update?.hasUpdate
                            ? '有可用更新，请查看下方更新说明。'
                            : update
                              ? '当前已是最新版本，无需更新。'
                              : '等待检查更新。'}
                    </span>
                  </div>
                  {!updateChecking && updateCheckError && (
                    <p className="n-error-text">{updateCheckError}</p>
                  )}
                  {updateBanner}
                  {!updateChecking && updateCheckedAt && (
                    <p className="n-config-note">
                      上次检查：
                      {new Date(updateCheckedAt).toLocaleString('zh-CN', { hour12: false })}
                    </p>
                  )}
                </section>
                <section className="n-panel">
                  <div className="n-panel-heading">
                    <h2>数据目录</h2>
                  </div>
                  <div className="n-path-box">
                    <code>{status?.dataPath ?? '正在加载…'}</code>
                    <button
                      className="n-icon-button"
                      onClick={() => status && copy(status.dataPath)}
                      aria-label="复制数据路径"
                      title="复制路径"
                    >
                      <Copy size={16} />
                    </button>
                  </div>
                </section>
                <section className="n-panel">
                  <div className="n-panel-heading">
                    <h2>外观</h2>
                    <span>{themeLabel}</span>
                  </div>
                  <p className="n-config-note">
                    默认跟随系统外观。手动选择会保存在本机，切换后立即生效。
                  </p>
                  <div className="n-theme-options" role="group" aria-label="外观">
                    {(
                      [
                        ['system', '跟随系统'],
                        ['light', '浅色'],
                        ['dark', '深色'],
                      ] as const
                    ).map(([value, label]) => (
                      <button
                        key={value}
                        className={`n-button${theme === value ? ' n-primary' : ''}`}
                        aria-pressed={theme === value}
                        onClick={() => setTheme(value)}
                      >
                        {value === 'system' ? <Palette size={15} /> : null}
                        {value === 'light' ? <Sun size={15} /> : null}
                        {value === 'dark' ? <Moon size={15} /> : null}
                        {label}
                      </button>
                    ))}
                  </div>
                </section>
                <section className="n-panel n-about-panel">
                  <div className="n-panel-heading">
                    <h2>关于</h2>
                    <span>v{appVersion}</span>
                  </div>
                  <p className="n-config-note">
                    {APP_NAME} 是 CYAPI 官方的 AI 工作流工具箱，用于管理模型接入、Skill、MCP
                    与软件接入。
                  </p>
                  <div className="n-about-actions">
                    <button
                      className="n-button n-primary"
                      onClick={() => void openSite(OFFICIAL_SITE)}
                    >
                      <Globe2 size={15} />
                      访问官网
                    </button>
                    <button className="n-button" onClick={() => void openSite(DOWNLOAD_PAGE)}>
                      <Download size={15} />
                      前往下载页
                    </button>
                  </div>
                  <div className="n-path-box">
                    <code>{OFFICIAL_SITE}</code>
                    <button
                      className="n-icon-button"
                      onClick={() => copy(OFFICIAL_SITE)}
                      aria-label="复制官网地址"
                      title="复制地址"
                    >
                      <Copy size={16} />
                    </button>
                  </div>
                  <p className="n-about-copyright">{COPYRIGHT_TEXT}</p>
                </section>
              </>
            )}
          </div>
        </main>
      </div>

      {selectedCollection && status && (
        <CollectionDialog
          collection={selectedCollection.collection}
          status={status}
          initialLocation={selectedCollection.location}
          installedEntries={selectedCollection.installedEntries}
          run={collectionRun.run}
          onStart={(input) => void collectionRun.start(input)}
          onCancel={collectionRun.cancel}
          onClear={collectionRun.clear}
          onClose={() => {
            const background = collectionRun.run.running;
            setSelectedCollection(undefined);
            if (background) {
              setNotice({
                text: '套件任务仍在后台进行，可在页面顶部查看进度或取消。',
              });
            }
          }}
        />
      )}
      {selected && (
        <Modal
          title={
            completed
              ? `${completed.title} 已安装`
              : selected.distribution === 'source-only'
                ? selected.title
                : `安装 ${selected.title}`
          }
          onClose={() => setSelected(undefined)}
          wide
          busy={!!busy}
        >
          {selected.distribution === 'source-only' ? (
            <>
              <div className="n-install-dialog">
                <p>{selected.distributionReason || '请从官方渠道获取此 Skill。'}</p>
                <div className="n-path-box">
                  <code>{selected.sourceUrl}</code>
                </div>
                <button className="n-button" onClick={() => copy(selected.sourceUrl)}>
                  <Copy size={15} />
                  复制官方链接
                </button>
                {alert}
              </div>
              <div className="n-dialog-footer">
                <button className="n-button" onClick={() => setSelected(undefined)}>
                  关闭
                </button>
              </div>
            </>
          ) : completed ? (
            <div className="n-completed">
              <CheckCircle2 size={32} />
              <p>文件已安装。新建 {targetLabels[completed.target]} 会话后加载。</p>
              <div className="n-path-box">
                <code>{completed.path}</code>
                <button
                  className="n-icon-button"
                  onClick={() => copy(completed.path)}
                  aria-label="复制路径"
                >
                  <Copy size={16} />
                </button>
              </div>
              {completed.configuration && (
                <button
                  className="n-button n-primary"
                  onClick={() => {
                    setSelected(undefined);
                    setWorkspace(completed.target);
                    setPage('installed');
                    setConfiguration({ title: completed.title, recordId: completed.id });
                  }}
                >
                  <Settings2 size={15} />
                  配置
                </button>
              )}
              {alert}
              <button
                className="n-button n-primary"
                onClick={() => {
                  setSelected(undefined);
                  setWorkspace(completed.target);
                  setPage('installed');
                }}
              >
                查看已安装
              </button>
            </div>
          ) : (
            <>
              <div className="n-install-dialog">
                <p className="n-install-meta">
                  v{selected.version}
                  {selected.requirements.map((r) => (
                    <span key={r.kind}>
                      {r.kind === 'node' ? 'Node' : 'Python'} ≥ {r.minimum}
                    </span>
                  ))}
                </p>
                {!!selected.setupNotes?.length && (
                  <div className="n-setup-notes">
                    {selected.setupNotes.map((note) => (
                      <p key={note}>{note}</p>
                    ))}
                  </div>
                )}
                <div className="n-install-grid">
                  <div>
                    <label className="n-field-label" htmlFor="install-target">
                      安装到
                    </label>
                    <select
                      id="install-target"
                      value={target}
                      disabled={!!busy}
                      onChange={(e) => setTarget(e.target.value as typeof target)}
                    >
                      <option value="codex">Codex</option>
                      <option value="claude">Claude Code</option>
                      <option value="opencode">OpenCode</option>
                    </select>
                  </div>
                  <div>
                    <label className="n-field-label" htmlFor="install-scope">
                      范围
                    </label>
                    <select
                      id="install-scope"
                      value={scope}
                      disabled={!!busy}
                      onChange={(e) => setScope(e.target.value as typeof scope)}
                    >
                      <option value="global">全局</option>
                      <option value="project">项目</option>
                    </select>
                  </div>
                </div>
                {scope === 'project' && (
                  <div>
                    <label className="n-field-label" htmlFor="project-path">
                      项目路径
                    </label>
                    <input
                      id="project-path"
                      placeholder="项目文件夹的绝对路径"
                      value={projectPath}
                      disabled={!!busy}
                      onChange={(e) => setProjectPath(e.target.value)}
                    />
                  </div>
                )}
                <div className="n-plan">
                  <h3>
                    <FolderOpen size={15} />
                    安装位置
                  </h3>
                  <code className={planError ? 'n-error-text' : undefined}>
                    {plan?.path ??
                      (planError || (!nativeAvailable ? '请使用桌面版安装' : '正在计算…'))}
                  </code>
                  {plan?.installed ? (
                    <p className="n-ready" role="status">
                      已安装同名 Skill
                    </p>
                  ) : (
                    plan?.conflict && (
                      <p className="n-error-text" role="alert">
                        已有同名 Skill，请选择其他安装位置。
                      </p>
                    )
                  )}
                </div>
                {plan && plan.runtimes.length > 0 && (
                  <div className="n-plan">
                    <h3>
                      <Cpu size={15} />
                      运行环境
                    </h3>
                    {plan.runtimes.map((r) => (
                      <div className="n-runtime-plan" key={r.requirement.kind}>
                        {r.reuse ? (
                          <>
                            <strong>
                              {r.reuse.kind} {r.reuse.version} · 复用{sourceLabels[r.reuse.source]}
                            </strong>
                            <code>{r.reuse.path}</code>
                          </>
                        ) : r.fallback ? (
                          <strong>
                            准备共享 {r.requirement.kind} {r.fallback.version}
                          </strong>
                        ) : (
                          <strong className="n-error-text">
                            {r.error ||
                              `缺少 ${r.requirement.kind} ≥ ${r.requirement.minimum}，暂无环境包`}
                          </strong>
                        )}
                      </div>
                    ))}
                  </div>
                )}
                <details className="n-source-details">
                  <summary>来源与文件</summary>
                  <p>
                    <a href={selected.sourceUrl} target="_blank" rel="noreferrer">
                      GitHub
                    </a>{' '}
                    · {selected.revision.slice(0, 12)} · {selected.fileCount} 个文件
                    {selected.license && ` · ${selected.license}`}
                  </p>
                  <code>SHA256 {selected.sha256}</code>
                  <div className="n-markdown">
                    <ReactMarkdown>{selected.markdown}</ReactMarkdown>
                  </div>
                </details>
                {alert}
                {progress}
              </div>
              <div className="n-dialog-footer">
                <button
                  className="n-button"
                  onClick={() => openLogs(lastOperation.current)}
                  data-logs-trigger
                >
                  <SquareTerminal size={14} />
                  查看本次日志
                </button>
                <span>
                  {plan?.installed ? (
                    '已安装'
                  ) : selectedReady ? (
                    <>
                      <CheckCircle2 size={14} />
                      文件已就绪
                    </>
                  ) : (
                    '需先下载离线文件'
                  )}
                </span>
                <button
                  className="n-button"
                  disabled={!!busy}
                  onClick={() => setSelected(undefined)}
                >
                  取消
                </button>
                <button
                  className="n-button n-primary"
                  disabled={
                    !!busy ||
                    !nativeAvailable ||
                    !!plan?.installed ||
                    (selectedReady && (!plan || plan.conflict)) ||
                    plan?.runtimes.some((runtime) => !runtime.reuse && !runtime.fallback)
                  }
                  onClick={() => (selectedReady ? install() : download(selected))}
                >
                  {busy ? <LoaderCircle size={15} className="n-spin" /> : <Download size={15} />}
                  {plan?.installed ? '已安装' : selectedReady ? '离线安装' : '下载文件'}
                </button>
              </div>
            </>
          )}
        </Modal>
      )}
      {configuration && (
        <ConfigurationDialog target={configuration} onClose={() => setConfiguration(undefined)} />
      )}
      {remove && (
        <Modal title={`卸载 ${remove.title}`} onClose={() => setRemove(undefined)} busy={!!busy}>
          <div className="n-remove">
            <p>卸载后保留文件备份和共享环境。</p>
            <div className="n-path-box">
              <code>{remove.path}</code>
            </div>
            {alert}
            {progress}
          </div>
          <div className="n-dialog-footer">
            <button className="n-button" onClick={() => setRemove(undefined)} disabled={!!busy}>
              取消
            </button>
            <button className="n-button n-danger" disabled={!!busy} onClick={uninstall}>
              {busy ? '正在卸载…' : '备份并卸载'}
            </button>
          </div>
        </Modal>
      )}
      {help && (
        <Modal title="使用帮助" onClose={() => setHelp(false)}>
          <div className="n-help">
            <ol>
              <li>在设置中连接 Skill 服务端。</li>
              <li>选择 Skill，下载文件后离线安装。</li>
              <li>新建 Agent 会话，调用已安装的 Skill。</li>
            </ol>
            <p>My Image 首次使用需配置图片服务。</p>
            <div className="n-help-links">
              <div className="n-help-actions">
                <button className="n-button" onClick={() => void openSite(OFFICIAL_SITE)}>
                  <Globe2 size={15} />
                  官方网站
                </button>
                <button className="n-button" onClick={() => void openSite(DOWNLOAD_PAGE)}>
                  <Download size={15} />
                  前往下载页
                </button>
              </div>
              <span className="n-help-copyright">{COPYRIGHT_TEXT}</span>
            </div>
            <details className="n-source-details">
              <summary>开源许可</summary>
              <pre className="n-license">{thirdPartyNotices}</pre>
            </details>
          </div>
        </Modal>
      )}
      {logsOpen && <LogWindow operationId={logOperation} onClose={() => setLogsOpen(false)} />}
    </div>
  );
}
