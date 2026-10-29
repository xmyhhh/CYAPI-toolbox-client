import { useState } from 'react';
import type { FormEvent } from 'react';
import { Check, Eye, EyeOff, KeyRound, LoaderCircle, Pencil, Trash2, X } from 'lucide-react';
import { Modal } from '../../components/Primitives';
import { maskKey } from './keyStore';
import type { WorkflowsController } from './useWorkflows';

export function KeysDialog({
  controller,
  onClose,
}: {
  controller: WorkflowsController;
  onClose: () => void;
}) {
  const { keys, active } = controller;
  const [name, setName] = useState('');
  const [value, setValue] = useState('');
  const [reveal, setReveal] = useState(false);
  const [remember, setRemember] = useState(true);
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState<string | null>(null);
  const [editName, setEditName] = useState('');
  const [removing, setRemoving] = useState<string | null>(null);

  const run = async (action: () => Promise<unknown>) => {
    if (saving) return;
    setSaving(true);
    setError('');
    try {
      await action();
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : '操作失败，请稍后重试');
    } finally {
      setSaving(false);
    }
  };

  const submit = async (event: FormEvent) => {
    event.preventDefault();
    const entryName = name;
    const entryKey = value;
    await run(async () => {
      await controller.addAndActivate(entryName, entryKey, remember);
      setName('');
      setValue('');
      setReveal(false);
    });
  };

  return (
    <Modal
      title="工作流 Key"
      onClose={onClose}
      className="wfx-dialog wfx-keys-dialog"
      busy={saving}
      dismissWhileBusy
    >
      <div className="wfx-dialog-body wfx-key-layout">
        <section className="wfx-key-list">
          <div className="wfx-subhead">
            <h3>本机 Key</h3>
            <span>{keys.length} / 10</span>
          </div>
          {keys.length === 0 && (
            <p className="wfx-muted">
              还没有保存的 Key。添加后可在此切换，草稿、任务与费用按 Key 分开记录。
            </p>
          )}
          {keys.map((entry) => {
            const selected = active?.id === entry.id;
            return (
              <article key={entry.id} className={`wfx-key-row${selected ? ' active' : ''}`}>
                <span className="wfx-key-mark" aria-hidden="true">
                  <KeyRound size={16} />
                </span>
                <div className="wfx-key-info">
                  {editing === entry.id ? (
                    <form
                      className="wfx-key-rename"
                      onSubmit={(event) => {
                        event.preventDefault();
                        void run(async () => {
                          controller.renameKey(entry.id, editName);
                          setEditing(null);
                        });
                      }}
                    >
                      <input
                        value={editName}
                        onChange={(event) => setEditName(event.target.value)}
                        maxLength={40}
                        aria-label="Key 名称"
                        autoFocus
                      />
                      <button type="submit" className="wfx-icon" aria-label="保存名称">
                        <Check size={15} />
                      </button>
                      <button
                        type="button"
                        className="wfx-icon"
                        aria-label="取消重命名"
                        onClick={() => setEditing(null)}
                      >
                        <X size={15} />
                      </button>
                    </form>
                  ) : (
                    <>
                      <strong>
                        {entry.name}
                        {selected && <span className="wfx-badge active">已激活</span>}
                      </strong>
                      <code>{maskKey(entry.key)}</code>
                    </>
                  )}
                </div>
                <div className="wfx-key-actions">
                  {!selected && (
                    <button
                      className="wfx-button small"
                      disabled={saving}
                      onClick={() => void run(() => controller.activateStored(entry.id))}
                    >
                      激活
                    </button>
                  )}
                  {editing !== entry.id && (
                    <button
                      className="wfx-icon"
                      aria-label={`重命名 ${entry.name}`}
                      disabled={saving}
                      onClick={() => {
                        setEditing(entry.id);
                        setEditName(entry.name);
                      }}
                    >
                      <Pencil size={14} />
                    </button>
                  )}
                  {removing === entry.id ? (
                    <span className="wfx-inline-confirm">
                      确认移除？
                      <button
                        className="wfx-text danger"
                        disabled={saving}
                        onClick={() =>
                          void run(async () => {
                            await controller.removeKey(entry.id);
                            setRemoving(null);
                          })
                        }
                      >
                        移除
                      </button>
                      <button className="wfx-text" onClick={() => setRemoving(null)}>
                        取消
                      </button>
                    </span>
                  ) : (
                    <button
                      className="wfx-icon"
                      aria-label={`移除 ${entry.name}`}
                      disabled={saving}
                      onClick={() => setRemoving(entry.id)}
                    >
                      <Trash2 size={14} />
                    </button>
                  )}
                </div>
              </article>
            );
          })}
          {active && (
            <div className="wfx-key-current">
              <span>
                当前：<strong>{active.name}</strong> <code>{maskKey(active.key)}</code>
                {active.temporary && <span className="wfx-badge">临时</span>}
              </span>
              <button className="wfx-text" disabled={saving} onClick={() => void run(controller.deactivate)}>
                停用
              </button>
            </div>
          )}
        </section>

        <section className="wfx-key-add">
          <div className="wfx-subhead">
            <h3>添加 Key</h3>
          </div>
          <form onSubmit={submit}>
            <label className="wfx-field">
              <span>名称</span>
              <input
                value={name}
                onChange={(event) => setName(event.target.value)}
                maxLength={40}
                placeholder="例如：日常使用"
                disabled={saving}
              />
            </label>
            <label className="wfx-field">
              <span>API Key</span>
              <span className="wfx-secret">
                <input
                  type={reveal ? 'text' : 'password'}
                  autoComplete="off"
                  spellCheck={false}
                  maxLength={4096}
                  value={value}
                  onChange={(event) => setValue(event.target.value)}
                  placeholder="粘贴 API Key"
                  required
                  disabled={saving}
                />
                <button
                  type="button"
                  className="wfx-icon"
                  aria-label={reveal ? '隐藏 Key' : '显示 Key'}
                  onClick={() => setReveal(!reveal)}
                >
                  {reveal ? <EyeOff size={15} /> : <Eye size={15} />}
                </button>
              </span>
            </label>
            <label className="wfx-check">
              <input
                type="checkbox"
                checked={remember}
                onChange={(event) => setRemember(event.target.checked)}
                disabled={saving}
              />
              <span>
                保存到本机
                <small>仅写入客户端 localStorage，不发送给服务端保存</small>
              </span>
            </label>
            {error && (
              <p className="wfx-error" role="alert">
                {error}
              </p>
            )}
            <button className="wfx-button primary wide" type="submit" disabled={saving || !value.trim()}>
              {saving ? <LoaderCircle className="n-spin" size={15} /> : <KeyRound size={15} />}
              {remember ? '保存并激活' : '仅本次使用'}
            </button>
          </form>
          <p className="wfx-hint">
            Key 只在本机保存与切换；服务端只按每次请求的 Bearer 头校验，不建立设备会话。同时只能激活一个
            Key。
          </p>
        </section>
      </div>
    </Modal>
  );
}
