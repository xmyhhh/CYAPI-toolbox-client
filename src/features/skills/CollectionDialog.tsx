import { useEffect, useMemo, useRef, useState } from 'react';
import { CheckCircle2, LoaderCircle, Package, Trash2 } from 'lucide-react';
import { Modal } from '../../components/Primitives';
import { nativeAvailable, nativeCall } from '../../lib/api/native';
import type { NativeStatus } from '../../lib/api/native';
import { collectionEntries } from './collections';
import type { InstalledSkill, SkillCollection } from './collections';
import { previewCollection } from './collection-install';
import type { CollectionLocation, CollectionStep } from './collection-install';
import type { CollectionRunState } from './collection-run';
import { removableCollectionEntries } from './collection-run';
import type { CollectionRunInput } from './useCollectionRun';

export function CollectionDialog({
  collection,
  status,
  initialLocation,
  installedEntries,
  run,
  onStart,
  onCancel,
  onClear,
  onClose,
}: {
  collection: SkillCollection;
  status: NativeStatus;
  initialLocation: CollectionLocation;
  installedEntries?: InstalledSkill[];
  run: CollectionRunState;
  onStart: (input: CollectionRunInput) => void;
  onCancel: () => void;
  onClear: () => void;
  onClose: () => void;
}) {
  const [location, setLocation] = useState(initialLocation);
  const [mode, setMode] = useState<'install' | 'uninstall'>('install');
  const [steps, setSteps] = useState<CollectionStep[]>();
  const [error, setError] = useState('');
  const [cancelled, setCancelled] = useState(false);
  const [removedRecordIds, setRemovedRecordIds] = useState<string[]>([]);
  const [previewVersion, setPreviewVersion] = useState(0);
  const working = run.running;
  const currentCollection = status.skillGroups?.find((group) => group.id === collection.id);
  const entries = useMemo(
    () => collectionEntries(status.skills, currentCollection),
    [status.skills, currentCollection],
  );
  useEffect(() => {
    let stopped = false;
    setSteps(undefined);
    setError('');
    if (mode === 'uninstall') return;
    if (!nativeAvailable) return;
    const timer = setTimeout(() => {
      previewCollection(entries, status.installed, location, nativeCall)
        .then((next) => {
          if (!stopped) setSteps(next);
        })
        .catch((failure) => {
          if (!stopped) setError(String(failure));
        });
    }, 200);
    return () => {
      stopped = true;
      clearTimeout(timer);
    };
  }, [entries, status.installed, location, previewVersion, mode]);

  // A finished run changes what is on disk, so the plan has to be recomputed.
  const wasRunning = useRef(false);
  useEffect(() => {
    if (wasRunning.current && !run.running) {
      setPreviewVersion((version) => version + 1);
      setCancelled(run.cancelled);
      if (run.removal?.removedIds.length) {
        setRemovedRecordIds((previous) => [...new Set([...previous, ...run.removal!.removedIds])]);
      }
    }
    wasRunning.current = run.running;
  }, [run.running, run.cancelled, run.removal]);

  const installs = steps?.filter((step) => step.action === 'install').length ?? 0;
  const updates = steps?.filter((step) => step.action === 'update').length ?? 0;
  const blocked = steps?.some((step) => step.action === 'blocked');
  const remainingInstalled = (installedEntries ?? []).filter(
    ({ record }) => !removedRecordIds.includes(record.id),
  );
  const removableEntries = removableCollectionEntries(remainingInstalled, location.target);
  const localRemovalCount = removableEntries.filter((entry) => entry.managed === false).length;
  const retainedCount = remainingInstalled.length - removableEntries.length;
  const changeLocation = (next: Partial<CollectionLocation>) => {
    setLocation((previous) => ({ ...previous, ...next }));
    onClear();
  };
  const startInstall = () => {
    if (!steps || working || blocked || !(installs + updates)) return;
    setError('');
    onStart({
      mode: 'install',
      collectionId: collection.id,
      location,
      steps,
    });
  };
  const startRemoval = () => {
    if (!removableEntries.length || working) return;
    setError('');
    onStart({
      mode: 'uninstall',
      collectionId: collection.id,
      location,
      entries: remainingInstalled,
    });
  };
  const elapsed = useElapsed(run.running, run.startedAt);
  return (
    <Modal
      title={`管理 ${currentCollection?.title ?? collection.title}`}
      onClose={onClose}
      wide
      busy={working}
      dismissWhileBusy
    >
      <div className="n-install-dialog">
        {!currentCollection && <p role="alert">该套件已从目录移除，请关闭后重新选择。</p>}
        {installedEntries && (
          <div className="n-cyapi-mode" role="group" aria-label="技能套件操作">
            <button
              type="button"
              aria-pressed={mode === 'install'}
              disabled={working}
              onClick={() => {
                setMode('install');
                setError('');
              }}
            >
              批量安装 / 更新
            </button>
            <button
              type="button"
              aria-pressed={mode === 'uninstall'}
              disabled={working}
              onClick={() => {
                setMode('uninstall');
                setLocation(initialLocation);
                setError('');
              }}
            >
              批量卸载 ({removableEntries.length})
            </button>
          </div>
        )}
        {mode === 'install' && (
          <>
            <p className="n-config-note">
              整套包含 {entries.length}{' '}
              个模块。一次补齐缺失功能，更新工具箱管理的旧版本；本机已有的其他版本会保留。
            </p>
            <div className="n-install-grid">
              <label className="n-field-label">
                安装到
                <select
                  value={location.target}
                  disabled={working}
                  onChange={(event) =>
                    changeLocation({ target: event.target.value as CollectionLocation['target'] })
                  }
                >
                  <option value="codex">Codex</option>
                  <option value="claude">Claude Code</option>
                  <option value="opencode">OpenCode</option>
                </select>
              </label>
              <label className="n-field-label">
                范围
                <select
                  value={location.scope}
                  disabled={working}
                  onChange={(event) =>
                    changeLocation({ scope: event.target.value as CollectionLocation['scope'] })
                  }
                >
                  <option value="global">全局</option>
                  <option value="project">项目</option>
                </select>
              </label>
            </div>
            {location.scope === 'project' && (
              <label className="n-field-label">
                项目路径
                <input
                  value={location.projectPath}
                  placeholder="项目文件夹的绝对路径"
                  disabled={working}
                  onChange={(event) => changeLocation({ projectPath: event.target.value })}
                />
              </label>
            )}
            {!nativeAvailable && <p className="n-config-note">请使用桌面版安装套件。</p>}
            {nativeAvailable && !steps && !error && (
              <p role="status">正在检查安装位置与运行环境…</p>
            )}
            <div className="n-collection-plan">
              {(
                steps ??
                entries.map((entry): CollectionStep => ({ entry, action: 'keep', note: '' }))
              ).map((step) => (
                <div key={step.entry.skill.id}>
                  <div>
                    <strong>{step.entry.skill.title}</strong>
                    {step.plan && <code>{step.plan.path}</code>}
                  </div>
                  <span className={step.action === 'blocked' ? 'n-error-text' : undefined}>
                    {step.note}
                  </span>
                </div>
              ))}
            </div>
            <p className="n-config-note">
              共享规则随套件安装。需要账号或额外软件的功能，可展开模块查看各自的配置说明。
            </p>
            <a
              href={currentCollection?.sourceUrl ?? collection.sourceUrl}
              target="_blank"
              rel="noreferrer"
            >
              查看上游仓库
            </a>
            {run.result && (
              <div
                className={`n-alert ${run.result.failed ? 'n-error' : 'n-success'}`}
                role={run.result.failed ? 'alert' : 'status'}
              >
                <span>
                  已安装 {run.result.installed} 项、更新 {run.result.updated} 项、保留{' '}
                  {run.result.kept} 项。
                  {run.result.failed
                    ? `在「${run.result.failed.title}」处停止：${run.result.failed.message}。已完成的模块可继续使用，处理后可重试。`
                    : '请新建 Agent 会话使用。'}
                </span>
              </div>
            )}
            {cancelled && (
              <div className="n-alert" role="status">
                <span>任务已取消，已完成的模块保留在本机。</span>
              </div>
            )}
            {error && (
              <p className="n-error-text" role="alert">
                {error}
              </p>
            )}
            {run.error && !working && (
              <p className="n-error-text" role="alert">
                {run.error}
              </p>
            )}
            {working && (
              <p role="status">
                <LoaderCircle size={15} className="n-spin" />{' '}
                {run.cancelling ? '正在取消…' : run.progress}
                {elapsed > 0 && ` · 已用时 ${elapsed} 秒`}
              </p>
            )}
          </>
        )}
        {mode === 'uninstall' && (
          <>
            <p className="n-config-note">
              确认后将下方可卸载的 {removableEntries.length} 项模块移至备份，其中本机已有{' '}
              {localRemovalCount}{' '}
              项。不会永久删除文件或共享运行环境；受保护、文件缺失或不属于当前工作区的模块会保留。
            </p>
            {localRemovalCount > 0 && (
              <p className="n-config-note">
                请核对下方完整路径。本机已有模块移除后，读取同一目录的其他客户端也会受影响；符号链接或目录联接仅移动链接，不移动源目录。
              </p>
            )}
            <p className="n-config-note">
              备份保存在 {status.dataPath.replace(/[\\/]+$/, '')}
              /backups/，需要恢复时可将对应备份移回原路径。
            </p>
            {!nativeAvailable && <p className="n-config-note">请使用桌面版卸载套件。</p>}
            <div className="n-collection-plan">
              {remainingInstalled.map((entry) => {
                const removable = removableEntries.some(
                  ({ record }) => record.id === entry.record.id,
                );
                const note = entry.removalBlockedReason
                  ? `${entry.removalBlockedReason}，保留`
                  : !entry.exists
                    ? '文件已变化或缺失，保留'
                    : removable
                      ? entry.managed === false
                        ? '本机已有 · 将移至备份'
                        : '将移至备份'
                      : entry.record.target !== location.target
                        ? '不属于当前工作区，保留'
                        : '当前不可移除，保留';
                return (
                  <div key={entry.record.id}>
                    <div>
                      <strong>{entry.record.title}</strong>
                      <code>{entry.record.path}</code>
                    </div>
                    <span className={!removable ? 'n-config-note' : undefined}>{note}</span>
                  </div>
                );
              })}
              {!remainingInstalled.length && <p>套件模块已全部移至备份。</p>}
            </div>
            {run.removal && (
              <div
                className={`n-alert ${run.removal.failed ? 'n-error' : 'n-success'}`}
                role={run.removal.failed ? 'alert' : 'status'}
              >
                <span>
                  已移至备份 {run.removal.removed} 项，保留 {run.removal.retained} 项。
                  {run.removal.failed &&
                    `在「${run.removal.failed.title}」处停止：${run.removal.failed.message}。可重试剩余模块。`}
                  {run.removal.refreshError && `列表刷新失败：${run.removal.refreshError}`}
                </span>
              </div>
            )}
            {cancelled && (
              <div className="n-alert" role="status">
                <span>任务已取消，已移至备份的模块不再回退。</span>
              </div>
            )}
            {error && (
              <p className="n-error-text" role="alert">
                {error}
              </p>
            )}
            {run.error && !working && (
              <p className="n-error-text" role="alert">
                {run.error}
              </p>
            )}
            {working && (
              <p role="status">
                <LoaderCircle size={15} className="n-spin" />{' '}
                {run.cancelling ? '正在取消…' : run.progress}
                {elapsed > 0 && ` · 已用时 ${elapsed} 秒`}
              </p>
            )}
          </>
        )}
      </div>
      <div className="n-dialog-footer">
        <span>
          {working
            ? run.cancelling
              ? '正在停止任务…'
              : '任务进行中，可取消或放至后台继续'
            : mode === 'install'
              ? blocked
                ? '请先处理标记的安装问题'
                : installs + updates
                  ? `待安装 ${installs} 项 · 待更新 ${updates} 项`
                  : '所有模块均已就绪或已保留'
              : `${removableEntries.length} 项可卸载 · ${retainedCount} 项保留`}
        </span>
        {working ? (
          <>
            <button className="n-button" onClick={onClose}>
              放至后台
            </button>
            <button className="n-button n-danger" disabled={run.cancelling} onClick={onCancel}>
              {run.cancelling ? '正在取消…' : '取消任务'}
            </button>
          </>
        ) : (
          <>
            <button className="n-button" onClick={onClose}>
              关闭
            </button>
            <button
              className={`n-button ${mode === 'uninstall' ? 'n-danger' : 'n-primary'}`}
              disabled={
                !nativeAvailable ||
                (mode === 'install'
                  ? !steps || blocked || !(installs + updates)
                  : !removableEntries.length)
              }
              onClick={() => (mode === 'install' ? startInstall() : startRemoval())}
            >
              {mode === 'uninstall' ? (
                <Trash2 size={15} />
              ) : installs + updates ? (
                <Package size={15} />
              ) : (
                <CheckCircle2 size={15} />
              )}
              {mode === 'uninstall'
                ? `备份并卸载 ${removableEntries.length} 项`
                : updates
                  ? installs
                    ? '补齐并更新套件'
                    : '更新套件'
                  : installs
                    ? '安装整套'
                    : '已就绪'}
            </button>
          </>
        )}
      </div>
    </Modal>
  );
}

/** Counts the seconds a task has been running, so a slow step is not a freeze. */
function useElapsed(running: boolean, startedAt?: number) {
  const [, setTick] = useState(0);
  useEffect(() => {
    if (!running || !startedAt) return;
    const timer = setInterval(() => setTick((value) => value + 1), 1000);
    return () => clearInterval(timer);
  }, [running, startedAt]);
  if (!running || !startedAt) return 0;
  return Math.floor((Date.now() - startedAt) / 1000);
}
