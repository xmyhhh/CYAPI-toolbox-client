import { useCallback, useEffect, useRef, useState } from 'react';
import { cancelNativeOperation, nativeCall } from '../../lib/api/native';
import type { NativeStatus } from '../../lib/api/native';
import {
  collectionPhaseLabels,
  installCollection,
  isCollectionCancelled,
} from './collection-install';
import type {
  CollectionCall,
  CollectionLocation,
  CollectionProgress,
  CollectionResult,
  CollectionStep,
} from './collection-install';
import { idleCollectionRun, removeCollectionEntries } from './collection-run';
import type {
  CollectionRemovalResult,
  CollectionRunMode,
  CollectionRunState,
} from './collection-run';
import type { InstalledSkill } from './collections';

export interface CollectionRunInput {
  mode: CollectionRunMode;
  collectionId: string;
  location: CollectionLocation;
  steps?: CollectionStep[];
  entries?: InstalledSkill[];
}

export interface CollectionRunHandlers {
  /** Keeps the window-wide status bar in step with the running task. */
  onBusy: (message: string) => void;
  onStatus: (
    status: NativeStatus,
    target: CollectionLocation['target'],
    installedEntries: InstalledSkill[],
  ) => void;
  installedEntriesFor: (
    next: NativeStatus,
    collectionId: string,
    target: CollectionLocation['target'],
    scope: CollectionLocation['scope'],
    skillPath?: string,
  ) => InstalledSkill[];
  call?: CollectionCall;
}

/**
 * Owns a suite task outside of the dialog, so the work keeps running when the
 * dialog is closed and the user can always stop it.
 */
export function useCollectionRun(handlers: CollectionRunHandlers) {
  const [run, setRun] = useState<CollectionRunState>(idleCollectionRun);
  const controller = useRef<AbortController | undefined>(undefined);
  const runId = useRef(0);
  const handlersRef = useRef(handlers);
  handlersRef.current = handlers;
  const call = handlers.call ?? nativeCall;

  const cancel = useCallback(() => {
    controller.current?.abort();
    // Ask the native side to stop the download that is in flight as well.
    void cancelNativeOperation();
    setRun((current) => (current.running ? { ...current, cancelling: true } : current));
  }, []);

  const clear = useCallback(() => {
    setRun((current) => ({
      ...current,
      error: '',
      cancelled: false,
      result: undefined,
      removal: undefined,
    }));
  }, []);

  const start = useCallback(
    async (input: CollectionRunInput) => {
      controller.current?.abort();
      const abort = new AbortController();
      controller.current = abort;
      const current = handlersRef.current;
      const id = ++runId.current;
      // A task that was replaced must not write its outcome over the new one.
      const stale = () => runId.current !== id;
      const label = input.mode === 'install' ? '正在安装技能套件' : '正在备份并卸载技能套件';
      const startedAt = Date.now();
      let progress = '正在准备…';
      const statusMessage = () => {
        const seconds = Math.floor((Date.now() - startedAt) / 1000);
        return `${label} · ${progress} · 已用时 ${seconds} 秒`;
      };
      const report = (update: CollectionProgress) => {
        progress = `${update.index}/${update.total} · ${update.title} · ${collectionPhaseLabels[update.phase]}`;
        if (stale()) return;
        setRun((state) => ({ ...state, progress }));
        current.onBusy(statusMessage());
      };
      setRun({
        mode: input.mode,
        running: true,
        cancelling: false,
        cancelled: false,
        progress,
        error: '',
        startedAt,
      });
      current.onBusy(statusMessage());
      let result: CollectionResult | undefined;
      let removal: CollectionRemovalResult | undefined;
      let error = '';
      let cancelled = false;
      const entries = input.entries ?? [];
      const target =
        input.mode === 'install'
          ? input.location.target
          : (entries[0]?.record.target ?? input.location.target);
      const scope =
        input.mode === 'install'
          ? input.location.scope
          : (entries[0]?.record.scope ?? input.location.scope);
      try {
        if (input.mode === 'install') {
          result = await installCollection(
            input.steps ?? [],
            input.location,
            call,
            report,
            abort.signal,
          );
        } else {
          const outcome = await removeCollectionEntries({
            entries,
            workspace: input.location.target,
            call,
            signal: abort.signal,
            onProgress: report,
          });
          removal = {
            removed: outcome.removed,
            retained: entries.length - outcome.removed,
            removedIds: outcome.removedIds,
            failed: outcome.failed,
          };
        }
      } catch (failure) {
        if (isCollectionCancelled(failure)) cancelled = true;
        else error = failure instanceof Error ? failure.message : String(failure);
      }
      // A stopped run still installed modules, so the list has to be refreshed.
      let refreshError: string | undefined;
      try {
        const next = await call<NativeStatus>('status');
        const skillPath =
          input.steps?.find((step) => step.plan)?.plan?.path ?? entries[0]?.record.path;
        current.onStatus(
          next,
          target,
          current.installedEntriesFor(next, input.collectionId, target, scope, skillPath),
        );
      } catch (failure) {
        refreshError = failure instanceof Error ? failure.message : String(failure);
      }
      if (refreshError) {
        if (removal) removal = { ...removal, refreshError };
        else
          error = error
            ? `${error}（列表刷新失败：${refreshError}）`
            : `列表刷新失败：${refreshError}`;
      }
      if (stale()) return;
      setRun((state) => ({
        ...state,
        running: false,
        cancelling: false,
        cancelled,
        progress: '',
        error,
        result,
        removal,
      }));
      current.onBusy('');
    },
    [call],
  );

  // The status bar repeats the elapsed time, so a slow download reads as
  // progress instead of a frozen window.
  const label = run.running
    ? run.mode === 'install'
      ? '正在安装技能套件'
      : '正在备份并卸载技能套件'
    : '';
  useEffect(() => {
    if (!run.running || !run.startedAt) return;
    const timer = setInterval(() => {
      const seconds = Math.floor((Date.now() - run.startedAt!) / 1000);
      handlersRef.current.onBusy(
        `${label} · ${run.progress || '正在准备…'} · 已用时 ${seconds} 秒`,
      );
    }, 1000);
    return () => clearInterval(timer);
  }, [run.running, run.startedAt, run.progress, label]);
  return { run, start, cancel, clear };
}
