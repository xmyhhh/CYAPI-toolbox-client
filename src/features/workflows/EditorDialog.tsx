import { useEffect, useRef, useState } from 'react';
import type { ChangeEvent } from 'react';
import {
  ArrowLeft,
  LoaderCircle,
  MonitorPlay,
  Play,
  Trash2,
  Upload,
  Volume2,
} from 'lucide-react';
import { Modal } from '../../components/Primitives';
import {
  uploadAudio,
  uploadReferenceImage,
  workflowApi,
} from './api';
import { feature, needsAudio } from './helpers';
import { AuthenticatedAudio, AuthenticatedImage } from './Media';
import type { WorkflowCall } from './useWorkflows';
import type { WorkflowDraft } from './types';

const errorText = (error: unknown) =>
  error instanceof Error ? error.message : '操作失败，请稍后重试';

export function EditorDialog({
  initial,
  keyName,
  apiKey,
  call,
  onClose,
  onQuote,
  onCanvas,
  notify,
}: {
  initial: WorkflowDraft;
  keyName: string;
  apiKey: string;
  call: WorkflowCall;
  onClose: () => void;
  onQuote: (draft: WorkflowDraft) => void;
  onCanvas: (id: string) => void;
  notify: (text: string, error?: boolean) => void;
}) {
  const [name, setName] = useState(initial.name);
  const [description, setDescription] = useState(initial.description || '');
  const [references, setReferences] = useState<string[]>(initial.references || []);
  const [audio, setAudio] = useState(initial.audio || 'sample');
  const [saving, setSaving] = useState(false);
  const [uploading, setUploading] = useState(false);
  const [status, setStatus] = useState('已保存到服务');
  const [error, setError] = useState('');
  const fields = useRef({ name, description, references, audio });
  fields.current = { name, description, references, audio };
  const dirty = useRef(false);
  const revisionRef = useRef(initial.revision);
  const timer = useRef<number | undefined>(undefined);
  const savingPromise = useRef<Promise<string> | null>(null);
  const files = useRef<HTMLInputElement>(null);
  const audioFile = useRef<HTMLInputElement>(null);
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
      window.clearTimeout(timer.current);
    };
  }, []);

  const markDirty = () => {
    dirty.current = true;
    setStatus('有未保存的修改');
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => {
      void saveNow().catch(() => {});
    }, 700);
  };

  async function saveNow(force = false): Promise<string> {
    if (!dirty.current && !force) return initial.id;
    if (savingPromise.current) {
      await savingPromise.current;
      if (dirty.current && force) return saveNow(force);
      return initial.id;
    }
    const snapshot = fields.current;
    setSaving(true);
    setError('');
    const promise = (async () => {
      const saved = await call((key) =>
        workflowApi.saveDraft(key, {
          id: initial.id,
          name: snapshot.name,
          description: snapshot.description,
          references: snapshot.references,
          audio: snapshot.audio,
          revision: revisionRef.current,
        }),
      );
      revisionRef.current = saved.revision;
      if (mounted.current) {
        dirty.current = false;
        setStatus('已保存到服务');
      }
      return saved.id;
    })();
    savingPromise.current = promise;
    try {
      return await promise;
    } catch (cause) {
      dirty.current = true;
      if (mounted.current) {
        setStatus('保存失败，请重试');
        setError(errorText(cause));
      }
      throw cause;
    } finally {
      savingPromise.current = null;
      if (mounted.current) setSaving(false);
    }
  }

  const close = () => {
    window.clearTimeout(timer.current);
    if (!dirty.current) {
      onClose();
      return;
    }
    void saveNow(true)
      .catch((cause) => notify(errorText(cause), true))
      .finally(onClose);
  };

  const uploadReferences = async (event: ChangeEvent<HTMLInputElement>) => {
    const selected = Array.from(event.target.files || []);
    event.target.value = '';
    if (!selected.length || uploading) return;
    if (references.length + selected.length > 6) {
      notify('最多上传 6 张参考图', true);
      return;
    }
    setUploading(true);
    setError('');
    try {
      const added: string[] = [];
      for (const file of selected) {
        if (file.size > 8 * 1024 * 1024) throw new Error('单张图片不能超过 8 MB');
        if (!['image/png', 'image/jpeg', 'image/webp'].includes(file.type))
          throw new Error('参考图仅支持 PNG、JPG、WebP');
        const uploaded = await call((key) => uploadReferenceImage(key, file));
        added.push(uploaded.id);
      }
      const next = [...fields.current.references, ...added];
      setReferences(next);
      fields.current = { ...fields.current, references: next };
      dirty.current = true;
      await saveNow(true);
      notify('参考图已保存');
    } catch (cause) {
      notify(errorText(cause), true);
    } finally {
      if (mounted.current) setUploading(false);
    }
  };

  const uploadVoice = async (event: ChangeEvent<HTMLInputElement>) => {
    const file = event.target.files?.[0];
    event.target.value = '';
    if (!file || uploading) return;
    if (file.size > 8 * 1024 * 1024) {
      notify('WAV 文件不能超过 8 MB', true);
      return;
    }
    setUploading(true);
    try {
      const uploaded = await call((key) => uploadAudio(key, file));
      setAudio(uploaded.id);
      fields.current = { ...fields.current, audio: uploaded.id };
      dirty.current = true;
      await saveNow(true);
      notify('语音已保存');
    } catch (cause) {
      notify(errorText(cause), true);
    } finally {
      if (mounted.current) setUploading(false);
    }
  };

  const run = async () => {
    try {
      setError('');
      await saveNow(true);
      onQuote({ ...initial, name: fields.current.name, revision: revisionRef.current });
    } catch (cause) {
      notify(errorText(cause), true);
    }
  };

  const openCanvas = async () => {
    try {
      await saveNow(true);
      onCanvas(initial.id);
    } catch (cause) {
      notify(errorText(cause), true);
    }
  };

  const info = feature(initial);
  const transcribe = needsAudio(initial);
  const busy = saving || uploading;

  return (
    <Modal
      title={initial.name}
      onClose={close}
      className="wfx-dialog wfx-editor-dialog"
      busy={busy}
      dismissWhileBusy
    >
      <div className="wfx-dialog-body wfx-editor">
        <section className="wfx-editor-main">
          <div className="wfx-editor-top">
            <button className="wfx-text" type="button" onClick={close}>
              <ArrowLeft size={14} /> 返回
            </button>
            <span className="wfx-badge">未启动</span>
          </div>
          <label className="wfx-field">
            <span>任务名称</span>
            <input
              value={name}
              maxLength={80}
              onChange={(event) => {
                setName(event.target.value);
                markDirty();
              }}
            />
          </label>

          <div className="wfx-field">
            <span>
              参考图片
              {initial.requiresReferences && <em className="wfx-required">必填</em>}
              <small>最多 6 张</small>
            </span>
            <div className="wfx-references">
              {references.map((id) => (
                <div className="wfx-reference" key={id}>
                  <AuthenticatedImage
                    path={`/api/workflows/uploads/${id}`}
                    apiKey={apiKey}
                    alt="参考图"
                  />
                  <button
                    type="button"
                    aria-label="移除参考图"
                    onClick={() => {
                      const next = fields.current.references.filter((item) => item !== id);
                      setReferences(next);
                      fields.current = { ...fields.current, references: next };
                      markDirty();
                    }}
                  >
                    <Trash2 size={13} />
                  </button>
                </div>
              ))}
              {references.length < 6 && (
                <button
                  type="button"
                  className="wfx-upload-tile"
                  disabled={busy}
                  onClick={() => files.current?.click()}
                >
                  {uploading ? <LoaderCircle className="n-spin" size={18} /> : <Upload size={18} />}
                  上传参考图
                </button>
              )}
            </div>
            <p className="wfx-hint">PNG / JPG / WebP，单张不超过 8 MB。</p>
            <input
              ref={files}
              type="file"
              accept="image/png,image/jpeg,image/webp"
              multiple
              hidden
              onChange={(event) => void uploadReferences(event)}
            />
          </div>

          {transcribe && (
            <div className="wfx-field">
              <span>语音输入</span>
              <p className="wfx-hint">
                {audio && audio !== 'sample' ? '已上传 WAV 音频。' : '使用内置示例语音。'}
                可上传 60 秒内、16 位 PCM WAV；Mock 返回固定转写样例。
              </p>
              <div className="wfx-inline-actions">
                <button
                  type="button"
                  className="wfx-button small"
                  disabled={busy}
                  onClick={() => audioFile.current?.click()}
                >
                  <Volume2 size={14} /> 上传 WAV
                </button>
                <button
                  type="button"
                  className="wfx-button small"
                  disabled={busy || audio === 'sample'}
                  onClick={() => {
                    setAudio('sample');
                    fields.current = { ...fields.current, audio: 'sample' };
                    markDirty();
                  }}
                >
                  使用示例语音
                </button>
              </div>
              {audio && audio !== 'sample' && (
                <AuthenticatedAudio path={`/api/workflows/uploads/${audio}`} apiKey={apiKey} />
              )}
              <input
                ref={audioFile}
                type="file"
                accept="audio/wav,audio/x-wav"
                hidden
                onChange={(event) => void uploadVoice(event)}
              />
            </div>
          )}

          <label className="wfx-field">
            <span>
              描述
              <small>{Array.from(description).length} / 4000</small>
            </span>
            <textarea
              value={description}
              maxLength={8000}
              placeholder="填写要生成的内容"
              rows={7}
              onChange={(event) => {
                setDescription(event.target.value);
                markDirty();
              }}
            />
          </label>

          {error && (
            <p className="wfx-error" role="alert">
              {error}
            </p>
          )}

          <div className="wfx-editor-footer">
            <span className="wfx-save-status" role="status">
              {uploading ? '正在上传…' : status}
            </span>
            <div className="wfx-inline-actions">
              <button
                type="button"
                className="wfx-button small"
                disabled={busy}
                onClick={() =>
                  void saveNow(true)
                    .then(() => notify('草稿已保存'))
                    .catch((cause) => notify(errorText(cause), true))
                }
              >
                保存草稿
              </button>
              <button type="button" className="wfx-button primary" disabled={busy} onClick={() => void run()}>
                {busy ? <LoaderCircle className="n-spin" size={15} /> : <Play size={15} />}
                启动任务
              </button>
            </div>
          </div>
        </section>

        <aside className="wfx-editor-aside">
          <h3>工作流信息</h3>
          <dl className="wfx-info">
            <div>
              <dt>模板版本</dt>
              <dd>v{initial.templateVersion}</dd>
            </div>
            <div>
              <dt>输出</dt>
              <dd>{info.output}</dd>
            </div>
            <div>
              <dt>当前 Key</dt>
              <dd>{keyName}</dd>
            </div>
          </dl>
          {info.steps.length > 0 && (
            <ol className="wfx-steps">
              {info.steps.map((step) => (
                <li key={step}>{step}</li>
              ))}
            </ol>
          )}
          <div className="wfx-aside-actions">
            <button type="button" className="wfx-button wide" disabled={busy} onClick={() => void openCanvas()}>
              <MonitorPlay size={15} /> 查看 / 编辑画布
            </button>
          </div>
          <p className="wfx-hint">
            画布是唯一从服务端加载的界面；编辑后保存会更新此草稿。启动任务始终以保存后的草稿为准。
          </p>
        </aside>
      </div>
    </Modal>
  );
}
