import { useEffect, useState } from 'react';
import { CheckCircle2, LoaderCircle } from 'lucide-react';
import { Modal } from '../components/Primitives';
import { nativeAvailable, nativeCall } from '../lib/api/native';
import type { ConfigFlow, ConfigurationView, ConfigValue } from '../lib/api/native';

export interface ConfigurationTarget {
  title: string;
  skillId?: string;
  recordId?: string;
  preview?: ConfigFlow | null;
}

/**
 * The path example of the browser preview.
 *
 * The native client shows the resolved absolute path. The browser preview
 * only knows the roots by name, so it uses the folder names of the system
 * the browser runs on.
 */
function pathExample(root: string, path: string): string {
  const agent = typeof navigator === 'undefined' ? '' : navigator.userAgent;
  const roots: Record<string, string> = agent.includes('Mac')
    ? {
        home: '~',
        appData: '~/Library/Application Support',
        localAppData: '~/Library/Caches',
        skill: '<Skill 目录>',
      }
    : agent.includes('Win')
      ? {
          home: '%USERPROFILE%',
          appData: '%APPDATA%',
          localAppData: '%LOCALAPPDATA%',
          skill: '<Skill 目录>',
        }
      : {
          home: '~',
          appData: '~/.local/share',
          localAppData: '~/.cache',
          skill: '<Skill 目录>',
        };
  const separator = agent.includes('Win') ? '\\' : '/';
  const base = roots[root] ?? '~';
  return `${base}${separator}${path.split('/').join(separator)}`;
}

export function ConfigurationDialog({
  target,
  onClose,
}: {
  target: ConfigurationTarget;
  onClose: () => void;
}) {
  const [view, setView] = useState<ConfigurationView>();
  const [values, setValues] = useState<Record<string, ConfigValue>>({});
  const [step, setStep] = useState(0);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState('');
  const [saved, setSaved] = useState(false);
  const options = { skillId: target.skillId, recordId: target.recordId };

  useEffect(() => {
    let active = true;
    setBusy(true);
    setView(undefined);
    setValues({});
    setStep(0);
    setError('');
    setSaved(false);
    async function load() {
      try {
        let next: ConfigurationView;
        if (nativeAvailable)
          next = await nativeCall<ConfigurationView>('configuration-read', options);
        else {
          if (!target.preview) throw new Error('此 Skill 未声明配置流程');
          next = {
            flow: target.preview,
            flowRevision: '',
            configurationRevision: '',
            configuredSecrets: {},
            values: Object.fromEntries(
              target.preview.steps
                .flatMap((s) => s.fields)
                .filter((f) => f.default !== undefined || f.type === 'boolean')
                .map((f) => [f.id, f.default !== undefined ? f.default : false]),
            ),
            files: target.preview.files.map((f) => ({
              path: pathExample(f.root, f.path),
              format: f.format,
              exists: false,
            })),
          };
        }
        if (active) {
          setView(next);
          setValues(next.values);
        }
      } catch (e) {
        if (active) setError(String(e instanceof Error ? e.message : e));
      } finally {
        if (active) setBusy(false);
      }
    }
    void load();
    return () => {
      active = false;
    };
  }, [target]);

  async function submit(event: React.FormEvent) {
    event.preventDefault();
    if (!view || busy) return;
    if (step < view.flow.steps.length - 1) {
      setStep(step + 1);
      return;
    }
    setBusy(true);
    setError('');
    try {
      await nativeCall('configuration-save', {
        ...options,
        values,
        flowRevision: view.flowRevision,
        configurationRevision: view.configurationRevision,
      });
      setSaved(true);
      setValues((current) =>
        Object.fromEntries(
          Object.entries(current).filter(
            ([id]) =>
              !view.flow.steps.some((step) =>
                step.fields.some((field) => field.id === id && field.type === 'secret'),
              ),
          ),
        ),
      );
      // Fetch only masked state after saving; discard the entered secret from form state.
      try {
        const next = await nativeCall<ConfigurationView>('configuration-read', options);
        setValues(next.values);
        setView(next);
      } catch (e) {
        setView(undefined);
        setError(
          `配置已保存，但重新读取失败，请关闭后重新打开：${e instanceof Error ? e.message : e}`,
        );
      }
    } catch (e) {
      setError(String(e instanceof Error ? e.message : e));
    } finally {
      setBusy(false);
    }
  }
  const current = view?.flow.steps[step];
  return (
    <Modal title={`配置 ${target.title}`} onClose={onClose} busy={busy}>
      <form className="n-config-form" onSubmit={submit} autoComplete="off">
        <div className="n-config-body">
          {busy && !view && (
            <p className="n-config-note" role="status">
              正在读取配置…
            </p>
          )}
          {current && (
            <>
              {view!.flow.steps.length > 1 && (
                <p className="n-config-step">
                  {step + 1} / {view!.flow.steps.length} · {current.title}
                </p>
              )}
              <div className="n-config-fields">
                {current.fields.map((field) => {
                  const id = `config-${field.id}`;
                  const preserved = field.type === 'secret' && view?.configuredSecrets[field.id];
                  const update = (value: ConfigValue) => {
                    setSaved(false);
                    setValues((v) => ({ ...v, [field.id]: value }));
                  };
                  return (
                    <div className="n-config-field" key={id}>
                      <label htmlFor={id}>
                        {field.label}
                        {field.required && <span aria-hidden="true"> *</span>}
                      </label>
                      {field.type === 'select' ? (
                        <select
                          id={id}
                          value={String(values[field.id] ?? '')}
                          required={field.required}
                          disabled={busy}
                          onChange={(e) => update(e.target.value)}
                        >
                          <option value="" disabled={field.required}>
                            请选择
                          </option>
                          {field.options?.map((o) => (
                            <option key={o} value={o}>
                              {o}
                            </option>
                          ))}
                        </select>
                      ) : field.type === 'boolean' ? (
                        <input
                          id={id}
                          type="checkbox"
                          checked={Boolean(values[field.id])}
                          disabled={busy}
                          onChange={(e) => update(e.target.checked)}
                        />
                      ) : (
                        <input
                          id={id}
                          type={
                            field.type === 'secret'
                              ? 'password'
                              : field.type === 'number'
                                ? 'number'
                                : field.type === 'url'
                                  ? 'url'
                                  : 'text'
                          }
                          value={String(values[field.id] ?? '')}
                          required={field.required && !preserved}
                          disabled={busy}
                          placeholder={preserved ? '已配置，留空保留原值' : field.placeholder}
                          min={field.minimum}
                          max={field.maximum}
                          maxLength={8192}
                          step={field.type === 'number' ? 'any' : undefined}
                          autoComplete={field.type === 'secret' ? 'new-password' : 'off'}
                          spellCheck={false}
                          onChange={(e) =>
                            update(
                              field.type === 'number'
                                ? e.target.value === ''
                                  ? null
                                  : Number(e.target.value)
                                : e.target.value,
                            )
                          }
                        />
                      )}
                    </div>
                  );
                })}
              </div>
              <div className="n-config-paths">
                <span>保存位置</span>
                {view!.files.map((file) => (
                  <code key={file.path}>{file.path}</code>
                ))}
              </div>
              <p className="n-config-note">
                {nativeAvailable ? '配置仅保存在本机。' : '网页预览 · 请在桌面版保存配置'}
              </p>
            </>
          )}
          {error && (
            <p className="n-error-text" role="alert">
              {error}
            </p>
          )}
          {saved && (
            <p className="n-config-success" role="status">
              <CheckCircle2 size={15} />
              已保存
            </p>
          )}
        </div>
        <div className="n-dialog-footer">
          {step > 0 && (
            <button
              type="button"
              className="n-button"
              disabled={busy}
              onClick={() => setStep(step - 1)}
            >
              上一步
            </button>
          )}
          <button type="button" className="n-button" disabled={busy} onClick={onClose}>
            {saved ? '完成' : '关闭'}
          </button>
          {!saved && (
            <button
              type="submit"
              className="n-button n-primary"
              disabled={busy || !view || (!nativeAvailable && step === view.flow.steps.length - 1)}
            >
              {busy && <LoaderCircle size={15} className="n-spin" />}
              {view && step < view.flow.steps.length - 1 ? '下一步' : '保存配置'}
            </button>
          )}
        </div>
      </form>
    </Modal>
  );
}
