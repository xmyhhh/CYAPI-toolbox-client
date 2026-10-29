import { useState } from 'react';
import type { FormEvent } from 'react';
import { LoaderCircle, MonitorPlay, Plus, Trash2 } from 'lucide-react';
import { Modal } from '../../components/Primitives';
import {
  DEMO_MESH,
  DEMO_TRANSCRIPT,
  DEMO_VIDEO,
  DEMO_VIDEO_POSTER,
  exampleFor,
  feature,
  priceLabel,
  type Example,
} from './helpers';
import { MeshPreview } from './Media';
import type { WorkflowTemplate } from './types';

function ExampleOutput({ example, apiKey }: { example: Example; apiKey?: string }) {
  if (example.kind === 'image')
    return <img className="wfx-example-image" src={example.image} alt={example.output} />;
  if (example.kind === 'video')
    return (
      <video
        className="wfx-media-video"
        src={DEMO_VIDEO}
        poster={DEMO_VIDEO_POSTER}
        controls
        loop
        playsInline
        aria-label="产品旋转演示视频"
      />
    );
  if (example.kind === 'mesh')
    return <MeshPreview path={DEMO_MESH} apiKey={apiKey} label="演示立方体 · 8 顶点 / 12 三角面" />;
  if (example.kind === 'text')
    return <pre className="wfx-transcript">[模拟转写样例]{`\n${DEMO_TRANSCRIPT}`}</pre>;
  return (
    <div className="wfx-example-gallery">
      <figure>
        <figcaption>1 · 转写文字</figcaption>
        <pre className="wfx-transcript">{DEMO_TRANSCRIPT}</pre>
      </figure>
      <figure>
        <figcaption>2 · 图片</figcaption>
        <img src={example.image} alt="产品场景图演示" />
      </figure>
      <figure>
        <figcaption>3 · 视频</figcaption>
        <video src={DEMO_VIDEO} poster={DEMO_VIDEO_POSTER} controls loop playsInline aria-label="演示视频" />
      </figure>
      <figure>
        <figcaption>4 · 3D 网格</figcaption>
        <MeshPreview path={DEMO_MESH} apiKey={apiKey} label="演示立方体" />
      </figure>
    </div>
  );
}

export function ExampleDialog({
  template,
  apiKey,
  onPreview,
  onUse,
  onClose,
}: {
  template: WorkflowTemplate;
  apiKey?: string;
  onPreview: () => void;
  onUse: () => void;
  onClose: () => void;
}) {
  const example = exampleFor(template);
  return (
    <Modal title="工作流示例" onClose={onClose} className="wfx-dialog wfx-example-dialog">
      <div className="wfx-dialog-body">
        {example ? (
          <>
            <h3>{example.title}</h3>
            <p className="wfx-hint">
              演示样例 · 非本次运行结果。用于说明输入与产物形式，本地 Mock 返回固定测试内容。
            </p>
            <h4>示例输入</h4>
            <p className="wfx-example-input">{example.input}</p>
            <h4>产物展示</h4>
            <ExampleOutput example={example} apiKey={apiKey} />
          </>
        ) : (
          <p>此工作流暂无演示样例。</p>
        )}
      </div>
      <div className="n-dialog-footer">
        <button type="button" className="wfx-button small" onClick={onPreview}>
          查看流程
        </button>
        <button type="button" className="wfx-button primary" onClick={onUse}>
          使用模板
        </button>
      </div>
    </Modal>
  );
}

export function PreviewDialog({
  template,
  billed,
  hasKey,
  onUse,
  onCanvas,
  onCopy,
  onExample,
  onClose,
}: {
  template: WorkflowTemplate;
  billed: boolean;
  hasKey: boolean;
  onUse: () => void;
  onCanvas: () => void;
  onCopy: () => void;
  onExample: () => void;
  onClose: () => void;
}) {
  const info = feature(template);
  const example = exampleFor(template);
  return (
    <Modal title="了解这个工作流" onClose={onClose} className="wfx-dialog wfx-preview-dialog">
      <div className="wfx-dialog-body">
        <header className="wfx-preview-heading">
          <h2>{template.name}</h2>
          <p>{template.summary}</p>
        </header>
        <dl className="wfx-info wfx-preview-facts">
          <div>
            <dt>输入</dt>
            <dd>{info.input}</dd>
          </div>
          <div>
            <dt>输出</dt>
            <dd>{info.output}</dd>
          </div>
          <div>
            <dt>参考费用</dt>
            <dd>
              {priceLabel(template, billed)}
              {billed ? '（测试余额）' : ''}
            </dd>
          </div>
        </dl>
        {info.steps.length > 0 && (
          <section className="wfx-preview-process">
            <h3>执行步骤</h3>
            <ol>
              {info.steps.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
          </section>
        )}
        {example && (
          <button type="button" className="wfx-example-card" onClick={onExample}>
            <img src={example.image} alt={example.output} />
            <span>
              <small>演示样例</small>
              <strong>{example.title}</strong>
            </span>
            <span className="wfx-example-link">查看 ›</span>
          </button>
        )}
        <p className="wfx-hint">{info.note}当前产物为 Mock 示例，实际费用以运行前的报价为准。</p>
      </div>
      <div className="n-dialog-footer">
        {hasKey && (
          <>
            <button type="button" className="wfx-button small" onClick={onCanvas}>
              <MonitorPlay size={14} /> 打开原生画布
            </button>
            <button type="button" className="wfx-button small" onClick={onCopy}>
              创建副本
            </button>
          </>
        )}
        <button type="button" className="wfx-button primary" onClick={onUse}>
          {hasKey ? '使用模板' : '激活 Key 后使用'}
        </button>
      </div>
    </Modal>
  );
}

export function GuideDialog({ onClose }: { onClose: () => void }) {
  return (
    <Modal title="使用指南" onClose={onClose} className="wfx-dialog wfx-guide-dialog">
      <div className="wfx-dialog-body">
        <ol className="wfx-guide-steps">
          {[
            ['选择模板', '在工作流库查看用途、输入要求和执行步骤，不需要 Key。'],
            ['填写输入', '在工作流库选择模板后填写描述、上传参考图或使用示例语音。'],
            ['激活 Key', '在 Key 管理中粘贴 API Key。Key 只保存在本机客户端，可添加多个并切换。'],
            [
              '确认运行',
              '点击启动任务，核对 Key 和报价后确认。任务在服务端继续执行，在结果页下载产物。',
            ],
          ].map(([title, text]) => (
            <li key={title}>
              <h3>{title}</h3>
              <p>{text}</p>
            </li>
          ))}
        </ol>
        <p className="wfx-hint">
          当前为本地 Mock 测试，产物为模拟样例。运行使用测试余额，浏览和配置不扣费。
        </p>
      </div>
    </Modal>
  );
}

export function SettingsDialog({
  mode,
  keyName,
  server,
  onKeys,
  onDeactivate,
  onClose,
}: {
  mode: string;
  keyName: string;
  server: string;
  onKeys: () => void;
  onDeactivate: () => void;
  onClose: () => void;
}) {
  return (
    <Modal title="工作台设置" onClose={onClose} className="wfx-dialog wfx-settings-dialog">
      <div className="wfx-dialog-body wfx-settings">
        <section>
          <h3>工作流扩展服务</h3>
          <p>
            <code>{server}</code>
          </p>
          <small>
            属于现有 Node 服务。关闭页面不会停止已提交任务；服务重启不自动重试。ComfyUI
            画布是唯一从服务端加载的界面。
          </small>
        </section>
        <section>
          <h3>API Key 登录</h3>
          <p>
            当前：<strong>{keyName}</strong>
          </p>
          <small>
            Key 只保存在本机客户端，不写入服务端配置。历史、参考图、草稿和结果按 Key
            隔离；重新输入同一 Key 可查询原结果，换 Key 不合并数据。
          </small>
          <div className="wfx-inline-actions">
            <button type="button" className="wfx-button small" onClick={onKeys}>
              管理工作流 Key
            </button>
            <button type="button" className="wfx-button small ghost" onClick={onDeactivate}>
              断开当前 Key
            </button>
          </div>
        </section>
        <section>
          <h3>当前边界</h3>
          <p>{mode === 'local-billing' ? '本地 Mock · 扣测试余额' : 'Mock · 不计费'}</p>
          <small>
            结果为参考图拼贴模拟，非 AI 生图。不读取 Agent
            凭据，不下载模型。单 Key 最多 24 张参考图；请尽快下载产物到本地。
          </small>
        </section>
      </div>
    </Modal>
  );
}

export function NewWorkflowDialog({
  templates,
  onCreate,
  onClose,
}: {
  templates: WorkflowTemplate[];
  onCreate: (name: string, baseId: string) => Promise<void>;
  onClose: () => void;
}) {
  const [name, setName] = useState('');
  const [base, setBase] = useState(templates[0]?.id || '');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const submit = async (event: FormEvent) => {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError('');
    try {
      await onCreate(name, base);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '创建失败，请稍后重试');
    } finally {
      setBusy(false);
    }
  };
  return (
    <Modal title="新建工作流" onClose={onClose} className="wfx-dialog" busy={busy} dismissWhileBusy>
      <form onSubmit={submit}>
        <div className="wfx-dialog-body">
          <label className="wfx-field">
            <span>工作流名称</span>
            <input
              value={name}
              maxLength={80}
              placeholder="为你的工作流起个名字"
              onChange={(event) => setName(event.target.value)}
              autoFocus
            />
          </label>
          <div className="wfx-new-options" role="radiogroup" aria-label="基础模板">
            {templates.map((template) => (
              <label key={template.id} className={base === template.id ? 'active' : ''}>
                <input
                  type="radio"
                  name="base"
                  value={template.id}
                  checked={base === template.id}
                  onChange={() => setBase(template.id)}
                />
                <span>
                  {template.name}
                  <small>{feature(template).input}</small>
                </span>
              </label>
            ))}
          </div>
          <p className="wfx-hint">创建后进入真正的 ComfyUI 画布编辑节点和连线。</p>
          {error && (
            <p className="wfx-error" role="alert">
              {error}
            </p>
          )}
        </div>
        <div className="n-dialog-footer">
          <button type="button" className="wfx-button small" onClick={onClose}>
            取消
          </button>
          <button type="submit" className="wfx-button primary" disabled={busy || !name.trim()}>
            {busy ? <LoaderCircle className="n-spin" size={14} /> : <Plus size={14} />}
            创建并打开画布
          </button>
        </div>
      </form>
    </Modal>
  );
}

export function DeleteTemplateDialog({
  template,
  onConfirm,
  onClose,
}: {
  template: WorkflowTemplate;
  onConfirm: () => Promise<void>;
  onClose: () => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  return (
    <Modal title="删除工作流" onClose={onClose} className="wfx-dialog" busy={busy} dismissWhileBusy>
      <div className="wfx-dialog-body">
        <p>确认删除「{template.name}」？</p>
        <p className="wfx-muted">只删除此个人模板，已有草稿、任务和产物仍然保留。需要备份可先导出。</p>
        {error && (
          <p className="wfx-error" role="alert">
            {error}
          </p>
        )}
      </div>
      <div className="n-dialog-footer">
        <button type="button" className="wfx-button small" onClick={onClose}>
          取消
        </button>
        <button
          type="button"
          className="wfx-button danger"
          disabled={busy}
          onClick={() => {
            setBusy(true);
            void onConfirm()
              .catch((cause) => setError(cause instanceof Error ? cause.message : '删除失败'))
              .finally(() => setBusy(false));
          }}
        >
          {busy ? <LoaderCircle className="n-spin" size={14} /> : <Trash2 size={14} />}
          确认删除
        </button>
      </div>
    </Modal>
  );
}
