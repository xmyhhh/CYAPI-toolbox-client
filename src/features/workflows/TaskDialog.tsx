import { useState } from 'react';
import { Download, LoaderCircle, MonitorPlay, Play } from 'lucide-react';
import { Modal } from '../../components/Primitives';
import { downloadArtifact } from './api';
import {
  billingLabels,
  formatDate,
  isActive,
  outputLabel,
  safeFilename,
  statusLabel,
  stepLabels,
} from './helpers';
import { AuthenticatedImage, OutputMedia } from './Media';
import type { WorkflowTask } from './types';

const filenameFrom = (path: string, index: number, name: string) => {
  const clean = path.split('?')[0];
  const base = clean.slice(clean.lastIndexOf('/') + 1);
  return /\.\w{2,5}$/.test(base) ? base : `${safeFilename(name)}-${index + 1}`;
};

export function TaskDialog({
  task,
  apiKey,
  onClose,
  onCancel,
  onCanvas,
  onCreateDraft,
  notify,
}: {
  task: WorkflowTask;
  apiKey: string;
  onClose: () => void;
  onCancel: (task: WorkflowTask) => Promise<void>;
  onCanvas: (task: WorkflowTask) => void;
  onCreateDraft: (templateId: string) => void;
  notify: (text: string, error?: boolean) => void;
}) {
  const [busy, setBusy] = useState(false);
  const active = isActive(task.status);
  const quote = task.quote;

  const cancel = async () => {
    setBusy(true);
    try {
      await onCancel(task);
      notify('取消请求已发送');
    } catch (cause) {
      notify(cause instanceof Error ? cause.message : '取消失败，请稍后重试', true);
    } finally {
      setBusy(false);
    }
  };

  const download = async (path: string, index: number) => {
    setBusy(true);
    try {
      await downloadArtifact(path, filenameFrom(path, index, task.name), apiKey);
    } catch (cause) {
      notify(cause instanceof Error ? cause.message : '下载失败，请稍后重试', true);
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={task.name}
      onClose={onClose}
      className="wfx-dialog wfx-task-dialog"
      busy={busy}
      dismissWhileBusy
    >
      <div className="wfx-dialog-body">
        <div className="wfx-task-meta">
          <span className={`wfx-badge status ${task.status}`}>{statusLabel(task.status)}</span>
          <small>
            {formatDate(task.createdAt)} · v{task.templateVersion} · Mock
          </small>
          <div className="wfx-inline-actions">
            <button type="button" className="wfx-button small" disabled={busy} onClick={() => onCanvas(task)}>
              <MonitorPlay size={14} /> 预览工作流
            </button>
            {active && (
              <button type="button" className="wfx-button small danger" disabled={busy} onClick={() => void cancel()}>
                {busy ? <LoaderCircle className="n-spin" size={14} /> : null}
                取消任务
              </button>
            )}
          </div>
        </div>

        <p className="wfx-description">{task.description || '（没有填写描述）'}</p>

        {task.references.length > 0 && (
          <div className="wfx-detail-refs">
            {task.references.map((id) => (
              <AuthenticatedImage
                key={id}
                path={`/api/workflows/uploads/${id}`}
                apiKey={apiKey}
                alt="参考图"
              />
            ))}
          </div>
        )}

        <p className="wfx-task-message" role="status">
          {task.message}
        </p>

        {quote && (
          <div className="wfx-task-quote">
            <span>
              {quote.mode === 'local-billing'
                ? `本地测试金额：${Number(quote.amount).toFixed(2)} USD · ${
                    billingLabels[task.billing?.status || ''] || '状态待核对'
                  }`
                : `模拟报价：${quote.amount} ${quote.unit}，未计费`}
            </span>
          </div>
        )}

        {task.steps && task.steps.length > 0 && (
          <ol className="wfx-task-steps">
            {task.steps.map((step) => (
              <li key={step.id} className={step.status}>
                <strong>{step.title}</strong>
                <span>{stepLabels[step.status] || step.status}</span>
              </li>
            ))}
          </ol>
        )}

        {task.outputs.length > 0 && (
          <>
            <div className="wfx-retention">
              <Download size={16} aria-hidden="true" />
              <div>
                <strong>请尽快下载到本地</strong>
                <p>超过 30 天的产物，服务端会不定时清理。</p>
              </div>
            </div>
            <div className="wfx-artifact-grid">
              {task.outputs.map((output, index) => (
                <article className="wfx-artifact-card" key={output.id}>
                  <header>
                    <h3>{output.title || '图片'}</h3>
                    <span>{outputLabel(output.kind)}</span>
                  </header>
                  <OutputMedia output={output} apiKey={apiKey} />
                  <button
                    type="button"
                    className="wfx-button primary small"
                    disabled={busy}
                    onClick={() => void download(output.download || output.url, index)}
                  >
                    <Download size={14} /> 下载产物
                  </button>
                </article>
              ))}
            </div>
          </>
        )}

        {!active && (
          <div className="wfx-dialog-actions">
            <button
              type="button"
              className="wfx-button small ghost"
              onClick={() => onCreateDraft(task.templateId)}
            >
              <Play size={14} /> 创建新草稿
            </button>
          </div>
        )}
      </div>
    </Modal>
  );
}
