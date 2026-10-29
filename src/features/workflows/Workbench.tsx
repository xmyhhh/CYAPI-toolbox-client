import { useCallback, useEffect, useRef, useState } from 'react';
import {
  BookOpen,
  Box,
  Download,
  FileText,
  Image as ImageIcon,
  Images,
  KeyRound,
  LoaderCircle,
  Pencil,
  Plus,
  RefreshCw,
  Search,
  Settings2,
  Trash2,
  Upload,
  Video,
  Wifi,
  WifiOff,
  Workflow as WorkflowIcon,
} from 'lucide-react';
import {
  WORKFLOW_ORIGIN,
  downloadArtifact,
  exportShare,
  importShare,
  workflowApi,
} from './api';
import { CanvasDialog } from './CanvasDialog';
import {
  DeleteTemplateDialog,
  ExampleDialog,
  GuideDialog,
  NewWorkflowDialog,
  PreviewDialog,
  SettingsDialog,
} from './Dialogs';
import { EditorDialog } from './EditorDialog';
import {
  countActive,
  exampleFor,
  feature,
  formatDate,
  isActive,
  libraryItems,
  priceLabel,
  safeFilename,
  statusLabel,
  templateCover,
} from './helpers';
import { KeysDialog } from './KeysDialog';
import { AuthenticatedImage, OutputMedia } from './Media';
import { QuoteDialog } from './QuoteDialog';
import { TaskDialog } from './TaskDialog';
import { useWorkflows } from './useWorkflows';
import type {
  WorkflowDraft,
  WorkflowQuote,
  WorkflowTask,
  WorkflowTemplate,
} from './types';
import type { CreationSection } from '../../lib/workbenches';
import './workflows.css';

type Dialog =
  | { type: 'keys' }
  | { type: 'guide' }
  | { type: 'settings' }
  | { type: 'preview'; template: WorkflowTemplate }
  | { type: 'example'; template: WorkflowTemplate }
  | { type: 'editor'; draft: WorkflowDraft }
  | {
      type: 'quote';
      quote: WorkflowQuote;
      draftId: string;
      draftName: string;
      editorId: string;
    }
  | { type: 'task'; task: WorkflowTask }
  | {
      type: 'canvas';
      kind: 'templates' | 'drafts' | 'tasks';
      id: string;
      preview: boolean;
      title: string;
      editorId?: string;
    }
  | { type: 'new' }
  | { type: 'delete'; template: WorkflowTemplate };

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : '操作失败，请稍后重试';

function FeatureIcon({ name }: { name: string }) {
  const props = { size: 17, strokeWidth: 1.9 } as const;
  if (name === 'image') return <ImageIcon {...props} />;
  if (name === 'video') return <Video {...props} />;
  if (name === 'mesh') return <Box {...props} />;
  if (name === 'text') return <FileText {...props} />;
  if (name === 'composite') return <Images {...props} />;
  return <WorkflowIcon {...props} />;
}

function TaskThumb({ task, apiKey }: { task: WorkflowTask; apiKey: string }) {
  const image = task.outputs.find((output) => !output.kind || output.kind === 'image');
  if (image && apiKey)
    return (
      <AuthenticatedImage
        path={image.url}
        apiKey={apiKey}
        alt={task.name}
        className="wfx-task-thumb"
      />
    );
  return <img className="wfx-task-thumb" src={templateCover(task)} alt="" />;
}

function EmptyState({
  title,
  text,
  action,
  onAction,
}: {
  title: string;
  text: string;
  action?: string;
  onAction?: () => void;
}) {
  return (
    <div className="wfx-empty">
      <WorkflowIcon size={26} strokeWidth={1.6} />
      <h3>{title}</h3>
      <p>{text}</p>
      {action && onAction && (
        <button type="button" className="wfx-button primary" onClick={onAction}>
          {action}
        </button>
      )}
    </div>
  );
}

function ActivationNotice({ onKeys }: { onKeys: () => void }) {
  return (
    <div className="wfx-activation">
      <h2>未激活 Key</h2>
      <p>激活后可以创建草稿、查看对应 Key 的任务与结果。浏览工作流库不需要 Key。</p>
      <div className="wfx-inline-actions">
        <button type="button" className="wfx-button primary" onClick={onKeys}>
          <KeyRound size={15} /> 前往 Key 管理
        </button>
      </div>
    </div>
  );
}

export function Workbench({
  initialSection,
  navigation,
  onSectionChange,
  onLockedChange,
}: {
  initialSection: CreationSection;
  navigation?: { section: CreationSection; id: number };
  onSectionChange: (section: CreationSection) => void;
  onLockedChange: (locked: boolean) => void;
  theme: string;
  visible: boolean;
}) {
  const controller = useWorkflows();
  const {
    snapshot,
    active,
    activeKey,
    mode,
    online,
    ready,
    busy,
    notice,
    notify,
    refresh,
    call,
    deactivate,
  } = controller;
  const [section, setSection] = useState<CreationSection>(initialSection);
  const [filter, setFilter] = useState('');
  const [query, setQuery] = useState('');
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const [downloading, setDownloading] = useState<string | null>(null);
  const pending = useRef<WorkflowTemplate | null>(null);
  const importFile = useRef<HTMLInputElement>(null);
  const billed = mode === 'local-billing';
  const keyName = active ? `${active.name}` : '未激活';

  const changeSection = useCallback(
    (next: CreationSection) => {
      setSection(next);
      onSectionChange(next);
    },
    [onSectionChange],
  );

  const navigationId = navigation?.id;
  useEffect(() => {
    if (!navigation) return;
    setSection(navigation.section);
    onSectionChange(navigation.section);
    // The navigation prop is a one-shot request identified by `id`.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [navigationId, onSectionChange]);

  const locked = Boolean(dialog) || busy;
  useEffect(() => {
    onLockedChange(locked);
  }, [locked, onLockedChange]);
  useEffect(() => () => onLockedChange(false), [onLockedChange]);

  useEffect(() => {
    if (!snapshot.categories.length) return;
    if (!snapshot.categories.some((category) => category.id === filter))
      setFilter(snapshot.categories[0].id);
  }, [snapshot.categories, filter]);

  async function createDraftFrom(template: WorkflowTemplate) {
    try {
      const draft = await call((key) => workflowApi.createDraft(key, template.id));
      setDialog({ type: 'editor', draft });
      void refresh(undefined, true);
    } catch (cause) {
      notify(errorText(cause), true);
    }
  }

  useEffect(() => {
    const template = pending.current;
    if (!active || !template) return;
    pending.current = null;
    setDialog(null);
    void createDraftFrom(template);
    // Only continue once a Key becomes active.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [activeKey]);

  const requireKey = () => {
    if (active) return true;
    notify('请先激活工作流 Key', true);
    setDialog({ type: 'keys' });
    return false;
  };

  const startTemplate = (template: WorkflowTemplate) => {
    if (!active) {
      pending.current = template;
      setDialog({ type: 'keys' });
      notify('激活 Key 后将打开此模板');
      return;
    }
    void createDraftFrom(template);
  };

  const openDraft = (id: string) => {
    const draft = snapshot.drafts.find((item) => item.id === id);
    if (!draft) {
      notify('草稿不存在，请刷新', true);
      return;
    }
    setDialog({ type: 'editor', draft: structuredClone(draft) });
  };

  const quoteDraft = async (draft: WorkflowDraft) => {
    try {
      const quote = await call((key) => workflowApi.quote(key, draft.id));
      setDialog({
        type: 'quote',
        quote,
        draftId: draft.id,
        draftName: draft.name,
        editorId: draft.id,
      });
    } catch (cause) {
      notify(errorText(cause), true);
    }
  };

  const confirmQuote = async (quote: WorkflowQuote, draftId: string) => {
    const task = await call((key) => workflowApi.run(key, draftId, quote));
    changeSection('queue');
    await refresh(undefined, true);
    setDialog({ type: 'task', task });
    notify('任务已加入队列');
  };

  const cancelTask = async (task: WorkflowTask) => {
    await call((key) => workflowApi.cancel(key, task.id));
    await refresh(undefined, true);
  };

  const openCanvas = (
    kind: 'templates' | 'drafts' | 'tasks',
    id: string,
    preview: boolean,
    title: string,
    editorId?: string,
  ) => setDialog({ type: 'canvas', kind, id, preview, title, editorId });

  const closeCanvas = async (current: Dialog) => {
    if (current.type !== 'canvas') return;
    setDialog(null);
    const next = await refresh(undefined, true);
    if (current.editorId) {
      const fresh = next?.drafts.find((draft) => draft.id === current.editorId);
      if (fresh) setDialog({ type: 'editor', draft: fresh });
    }
  };

  const closeQuote = async (current: Dialog) => {
    if (current.type !== 'quote') return;
    setDialog(null);
    if (!current.editorId || !snapshot.drafts.some((draft) => draft.id === current.editorId)) return;
    const next = await refresh(undefined, true);
    const fresh = next?.drafts.find((draft) => draft.id === current.editorId);
    if (fresh) setDialog({ type: 'editor', draft: fresh });
  };

  const createTemplate = async (name: string, baseId: string) => {
    const item = await call((key) => workflowApi.createTemplate(key, { name, fromId: baseId }));
    await refresh(undefined, true);
    notify(`已创建「${item.name}」`);
    openCanvas('templates', item.id, false, 'ComfyUI · 编辑画布');
  };

  const copyTemplate = async (template: WorkflowTemplate) => {
    if (!requireKey()) return;
    try {
      const item = await call((key) =>
        workflowApi.createTemplate(key, { name: `${template.name} · 副本`, fromId: template.id }),
      );
      await refresh(undefined, true);
      notify(`已创建「${item.name}」`);
      openCanvas('templates', item.id, false, 'ComfyUI · 编辑画布');
    } catch (cause) {
      notify(errorText(cause), true);
    }
  };

  const removeTemplate = async (template: WorkflowTemplate) => {
    await call((key) => workflowApi.deleteTemplate(key, template.id));
    setDialog(null);
    await refresh(undefined, true);
    notify('工作流已删除');
  };

  const exportTemplate = async (template: WorkflowTemplate) => {
    try {
      await call((key) => exportShare(key, template.id, safeFilename(template.name)));
      notify('已开始下载工作流，可通过“导入”恢复');
    } catch (cause) {
      notify(errorText(cause), true);
    }
  };

  const runImport = async (file: File) => {
    if (!requireKey()) return;
    try {
      const item = await call((key) => importShare(key, file));
      await refresh(undefined, true);
      notify(`已导入「${item.name}」`);
    } catch (cause) {
      notify(errorText(cause), true);
    }
  };

  const downloadOutput = async (outputId: string, path: string) => {
    if (!activeKey) return;
    setDownloading(outputId);
    try {
      await downloadArtifact(path, outputId, activeKey);
    } catch (cause) {
      notify(errorText(cause), true);
    } finally {
      setDownloading(null);
    }
  };

  const templates = snapshot.templates;
  const drafts = snapshot.drafts.filter((item) => !item.taskId);
  const tasks = [...snapshot.tasks].sort((a, b) => b.createdAt - a.createdAt);
  const running = countActive(snapshot.tasks);
  const library = libraryItems(snapshot.categories, templates, filter, query);
  const personal = templates.filter((item) => item.source === 'personal');

  return (
    <div className="wfx">
      <header className="wfx-top">
        <div className="wfx-top-status">
          <span className={`wfx-connection${online ? ' on' : ''}`} role="status">
            {online ? <Wifi size={13} /> : <WifiOff size={13} />}
            {online ? '服务已连接' : '服务连接已断开'}
          </span>
          <span className="wfx-mode">{billed ? '本地测试 · 扣测试余额' : 'Mock · 不计费'}</span>
        </div>
        <div className="wfx-top-actions">
          <button
            type="button"
            className={`wfx-key-chip${active ? ' active' : ''}`}
            onClick={() => setDialog({ type: 'keys' })}
            title="管理工作流 Key"
          >
            <KeyRound size={14} />
            {active ? active.name : '激活 Key'}
            {active && <small>{active.key.slice(-4)}</small>}
          </button>
          <button type="button" className="wfx-text" onClick={() => setDialog({ type: 'guide' })}>
            <BookOpen size={14} /> 使用指南
          </button>
          <button type="button" className="wfx-text" onClick={() => setDialog({ type: 'settings' })}>
            <Settings2 size={14} /> 设置
          </button>
          <button
            type="button"
            className="wfx-icon"
            aria-label="刷新"
            title="刷新"
            disabled={busy}
            onClick={() => void refresh()}
          >
            <RefreshCw size={15} className={busy ? 'n-spin' : ''} />
          </button>
        </div>
      </header>

      {!online && (
        <div className="wfx-offline" role="status">
          服务连接已断开，正在重新连接…
        </div>
      )}

      <div className="wfx-scroll">
        {!ready ? (
          <div className="wfx-loading" role="status">
            <LoaderCircle className="n-spin" size={22} /> 正在连接工作流服务…
          </div>
        ) : section === 'library' ? (
          <>
            <header className="wfx-heading">
              <div>
                <h2>工作流库</h2>
                <p>选择模板，填写输入，确认费用后运行。可先查看流程。</p>
              </div>
              <div className="wfx-inline-actions">
                <button
                  type="button"
                  className="wfx-button small"
                  disabled={!active}
                  onClick={() => importFile.current?.click()}
                >
                  <Upload size={14} /> 导入
                </button>
                <button
                  type="button"
                  className="wfx-button small"
                  onClick={() => {
                    if (!requireKey()) return;
                    if (!templates.some((item) => item.source === 'server')) {
                      notify('暂无可用的基础模板', true);
                      return;
                    }
                    setDialog({ type: 'new' });
                  }}
                >
                  <Plus size={14} /> 新建工作流
                </button>
              </div>
            </header>

            <div className="wfx-toolbar">
              <div className="wfx-filters" role="group" aria-label="工作流分类">
                {snapshot.categories.map((category) => (
                  <button
                    key={category.id}
                    type="button"
                    aria-pressed={filter === category.id}
                    className={`wfx-filter${filter === category.id ? ' active' : ''}`}
                    onClick={() => setFilter(category.id)}
                  >
                    {category.label}
                  </button>
                ))}
              </div>
              <label className="wfx-search">
                <Search size={14} aria-hidden="true" />
                <input
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                  placeholder="搜索工作流"
                  aria-label="搜索工作流"
                />
              </label>
            </div>

            <div className="wfx-grid">
              {library.map((template) => {
                const info = feature(template);
                const example = template.source === 'server' ? template.id : '';
                return (
                  <article className="wfx-card" key={template.id}>
                    <header>
                      <span className="wfx-card-icon">
                        <FeatureIcon name={info.icon} />
                      </span>
                      <div>
                        <h3>{template.name}</h3>
                        <small>
                          {template.source === 'personal' ? '个人模板' : info.label || template.category} · v
                          {template.version}
                        </small>
                      </div>
                    </header>
                    <p className="wfx-summary" title={template.summary}>
                      {template.summary}
                    </p>
                    {example && (
                      <button
                        type="button"
                        className="wfx-example-card"
                        onClick={() => setDialog({ type: 'example', template })}
                      >
                        <img src={exampleFor(template)?.image || templateCover(template)} alt="" />
                        <span>
                          <small>演示样例</small>
                          <strong>查看示例</strong>
                        </span>
                        <span className="wfx-example-link">查看 ›</span>
                      </button>
                    )}
                    <dl className="wfx-facts">
                      <div>
                        <dt>输入</dt>
                        <dd>{info.input}</dd>
                      </div>
                      <div>
                        <dt>输出</dt>
                        <dd>{info.output}</dd>
                      </div>
                    </dl>
                    <footer>
                      <span className="wfx-price">
                        {billed && info.price ? (
                          <>
                            <strong>{Number(info.price.amount).toFixed(2)}</strong> {info.price.currency} /{' '}
                            {info.price.unit}
                          </>
                        ) : (
                          priceLabel(template, billed)
                        )}
                      </span>
                      <div className="wfx-inline-actions">
                        <button
                          type="button"
                          className="wfx-button small"
                          onClick={() => setDialog({ type: 'preview', template })}
                        >
                          查看流程
                        </button>
                        <button
                          type="button"
                          className="wfx-button small primary"
                          onClick={() => startTemplate(template)}
                        >
                          使用模板
                        </button>
                      </div>
                    </footer>
                    {template.source === 'personal' && (
                      <div className="wfx-card-manage">
                        <button
                          type="button"
                          className="wfx-text"
                          onClick={() => void exportTemplate(template)}
                        >
                          <Download size={13} /> 导出
                        </button>
                        <button
                          type="button"
                          className="wfx-text"
                          onClick={() => setDialog({ type: 'preview', template })}
                        >
                          <Pencil size={13} /> 画布
                        </button>
                        <button
                          type="button"
                          className="wfx-text danger"
                          onClick={() => setDialog({ type: 'delete', template })}
                        >
                          <Trash2 size={13} /> 删除
                        </button>
                      </div>
                    )}
                  </article>
                );
              })}
              {library.length === 0 && (
                <EmptyState
                  title="没有找到匹配的工作流"
                  text="换个分类或清除搜索词后再试。"
                  action="清除搜索"
                  onAction={() => setQuery('')}
                />
              )}
            </div>
            {personal.length > 0 && (
              <p className="wfx-footnote">
                个人工作流 {personal.length} 个 · 分享文件不包含描述或参考图。
              </p>
            )}
          </>
        ) : section === 'drafts' ? (
          <>
            <header className="wfx-heading">
              <div>
                <h2>草稿</h2>
                <p>准备好素材再启动，草稿不会自动执行。</p>
              </div>
              <span className="wfx-count">{drafts.length} 个</span>
            </header>
            {!active ? (
              <ActivationNotice onKeys={() => setDialog({ type: 'keys' })} />
            ) : drafts.length === 0 ? (
              <EmptyState
                title="还没有草稿"
                text="先选择一个工作流，为这次创作准备素材。"
                action="去选择工作流"
                onAction={() => changeSection('library')}
              />
            ) : (
              <div className="wfx-list">
                {[...drafts]
                  .sort((a, b) => b.updatedAt - a.updatedAt)
                  .map((draft) => (
                    <article className="wfx-list-item" key={draft.id}>
                      <img className="wfx-cover" src={templateCover(draft)} alt="" />
                      <div className="wfx-grow">
                        <h3>{draft.name}</h3>
                        <p>{draft.description || '还未填写描述'}</p>
                        <small>
                          {draft.references.length} 张参考图 · {formatDate(draft.updatedAt)}
                        </small>
                      </div>
                      <button
                        type="button"
                        className="wfx-button small"
                        onClick={() => openDraft(draft.id)}
                      >
                        继续编辑
                      </button>
                    </article>
                  ))}
              </div>
            )}
          </>
        ) : section === 'queue' ? (
          <>
            <header className="wfx-heading">
              <div>
                <h2>
                  任务队列 <small>{running} 个进行中</small>
                </h2>
                <p>已提交的任务在服务中继续执行，关闭客户端也不受影响。</p>
              </div>
              <button
                type="button"
                className="wfx-button small"
                disabled={busy}
                onClick={() => void refresh()}
              >
                <RefreshCw size={14} /> 刷新
              </button>
            </header>
            {!active ? (
              <ActivationNotice onKeys={() => setDialog({ type: 'keys' })} />
            ) : tasks.length === 0 ? (
              <EmptyState
                title="暂无任务"
                text="启动的任务会显示在这里。"
                action="去选择工作流"
                onAction={() => changeSection('library')}
              />
            ) : (
              <div className="wfx-list">
                {tasks.map((task) => (
                  <article className="wfx-list-item" key={task.id}>
                    <TaskThumb task={task} apiKey={activeKey} />
                    <div className="wfx-grow">
                      <h3>{task.name}</h3>
                      <p>{task.description}</p>
                      <small>
                        {formatDate(task.createdAt)} · {task.message}
                      </small>
                    </div>
                    <span className={`wfx-badge status ${task.status}`}>{statusLabel(task.status)}</span>
                    <div className="wfx-inline-actions">
                      {isActive(task.status) && (
                        <button
                          type="button"
                          className="wfx-button small ghost"
                          disabled={busy}
                          onClick={() =>
                            void cancelTask(task).catch((cause) =>
                              notify(errorText(cause), true),
                            )
                          }
                        >
                          取消
                        </button>
                      )}
                      <button
                        type="button"
                        className="wfx-button small"
                        onClick={() => setDialog({ type: 'task', task })}
                      >
                        {task.status === 'succeeded' ? '查看结果' : '详情'}
                      </button>
                    </div>
                  </article>
                ))}
              </div>
            )}
          </>
        ) : (
          <>
            <header className="wfx-heading">
              <div>
                <h2>创作结果</h2>
                <p>预览生成的图片、视频、网格和文字，或下载到本地。</p>
              </div>
            </header>
            {!active ? (
              <ActivationNotice onKeys={() => setDialog({ type: 'keys' })} />
            ) : tasks.every((task) => task.outputs.length === 0) ? (
              <EmptyState
                title="暂无结果"
                text="任务完成后，在这里预览和下载产物。"
                action="去选择工作流"
                onAction={() => changeSection('library')}
              />
            ) : (
              <>
                <div className="wfx-retention">
                  <Download size={16} aria-hidden="true" />
                  <div>
                    <strong>请尽快下载到本地</strong>
                    <p>超过 30 天的产物，服务端会不定时清理。</p>
                  </div>
                </div>
                <div className="wfx-results">
                  {tasks.flatMap((task) =>
                    task.outputs.map((output) => (
                      <article className="wfx-result-card" key={`${task.id}-${output.id}`}>
                        <OutputMedia output={output} apiKey={activeKey} />
                        <div className="wfx-result-body">
                          <div className="wfx-result-title">
                            <h3>{task.name}</h3>
                            <span className="wfx-badge">{output.title || 'Mock 图片'}</span>
                          </div>
                          <p>{formatDate(task.createdAt)} · 非 AI 生成</p>
                          <div className="wfx-inline-actions">
                            <button
                              type="button"
                              className="wfx-button primary small"
                              disabled={downloading === output.id}
                              onClick={() =>
                                void downloadOutput(output.id, output.download || output.url)
                              }
                            >
                              {downloading === output.id ? (
                                <LoaderCircle className="n-spin" size={14} />
                              ) : (
                                <Download size={14} />
                              )}
                              下载产物
                            </button>
                            <button
                              type="button"
                              className="wfx-button small"
                              onClick={() => setDialog({ type: 'task', task })}
                            >
                              查看任务
                            </button>
                          </div>
                        </div>
                      </article>
                    )),
                  )}
                </div>
              </>
            )}
          </>
        )}
      </div>

      {notice && (
        <div className={`wfx-toast${notice.error ? ' error' : ''}`} role="status" key={notice.id}>
          {notice.text}
        </div>
      )}

      <input
        ref={importFile}
        type="file"
        accept="application/json,.json"
        hidden
        onChange={(event) => {
          const file = event.target.files?.[0];
          event.target.value = '';
          if (file) void runImport(file);
        }}
      />

      {dialog?.type === 'keys' && (
        <KeysDialog controller={controller} onClose={() => setDialog(null)} />
      )}
      {dialog?.type === 'guide' && <GuideDialog onClose={() => setDialog(null)} />}
      {dialog?.type === 'settings' && (
        <SettingsDialog
          mode={mode}
          keyName={keyName}
          server={WORKFLOW_ORIGIN}
          onKeys={() => setDialog({ type: 'keys' })}
          onDeactivate={() => {
            void deactivate().finally(() => setDialog(null));
          }}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.type === 'preview' && (
        <PreviewDialog
          template={dialog.template}
          billed={billed}
          hasKey={Boolean(active)}
          onUse={() => startTemplate(dialog.template)}
          onCanvas={() =>
            openCanvas(
              'templates',
              dialog.template.id,
              dialog.template.source !== 'personal',
              'ComfyUI · 预览画布',
            )
          }
          onCopy={() => void copyTemplate(dialog.template)}
          onExample={() => setDialog({ type: 'example', template: dialog.template })}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.type === 'example' && (
        <ExampleDialog
          template={dialog.template}
          apiKey={activeKey}
          onPreview={() => setDialog({ type: 'preview', template: dialog.template })}
          onUse={() => startTemplate(dialog.template)}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.type === 'editor' && active && (
        <EditorDialog
          initial={dialog.draft}
          apiKey={activeKey}
          keyName={keyName}
          call={call}
          onClose={() => {
            setDialog(null);
            void refresh(undefined, true);
          }}
          onQuote={(draft) => void quoteDraft(draft)}
          onCanvas={(id) => openCanvas('drafts', id, false, 'ComfyUI · 编辑画布', id)}
          notify={notify}
        />
      )}
      {dialog?.type === 'quote' && (
        <QuoteDialog
          quote={dialog.quote}
          draftName={dialog.draftName}
          keyName={keyName}
          onConfirm={() => confirmQuote(dialog.quote, dialog.draftId)}
          onClose={() => void closeQuote(dialog)}
        />
      )}
      {dialog?.type === 'task' &&
        (() => {
          const task = snapshot.tasks.find((item) => item.id === dialog.task.id) || dialog.task;
          return (
            <TaskDialog
              task={task}
              apiKey={activeKey}
              onClose={() => setDialog(null)}
              onCancel={cancelTask}
              onCanvas={(item) => openCanvas('tasks', item.id, true, 'ComfyUI · 预览画布')}
              onCreateDraft={(templateId) => {
                const template = templates.find((item) => item.id === templateId);
                setDialog(null);
                if (template) startTemplate(template);
                else notify('模板已不存在，请从工作流库重新选择', true);
              }}
              notify={notify}
            />
          );
        })()}
      {dialog?.type === 'canvas' && active && (
        <CanvasDialog
          kind={dialog.kind}
          id={dialog.id}
          preview={dialog.preview}
          apiKey={activeKey}
          title={dialog.title}
          onClose={() => void closeCanvas(dialog)}
        />
      )}
      {dialog?.type === 'new' && (
        <NewWorkflowDialog
          templates={templates.filter((item) => item.source === 'server')}
          onCreate={createTemplate}
          onClose={() => setDialog(null)}
        />
      )}
      {dialog?.type === 'delete' && (
        <DeleteTemplateDialog
          template={dialog.template}
          onConfirm={() => removeTemplate(dialog.template)}
          onClose={() => setDialog(null)}
        />
      )}
    </div>
  );
}
