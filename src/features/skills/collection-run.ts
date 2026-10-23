import { CollectionCancelledError, isCollectionCancelled } from './collection-install.ts';
import type { CollectionCall, CollectionProgress, CollectionResult } from './collection-install.ts';
import type { InstalledSkill } from './collections.ts';

export type CollectionRunMode = 'install' | 'uninstall';

export interface CollectionRemovalResult {
  removed: number;
  retained: number;
  /** The records that left the installation, so the list can hide them. */
  removedIds: string[];
  failed?: { title: string; message: string };
  refreshError?: string;
}

export interface CollectionRunState {
  mode: CollectionRunMode;
  running: boolean;
  /** The user asked to stop; the current native call still has to return. */
  cancelling: boolean;
  cancelled: boolean;
  progress: string;
  error: string;
  startedAt?: number;
  result?: CollectionResult;
  removal?: CollectionRemovalResult;
}

export const idleCollectionRun: CollectionRunState = {
  mode: 'install',
  running: false,
  cancelling: false,
  cancelled: false,
  progress: '',
  error: '',
};

/**
 * Move every module of a suite to the backup directory. The run stops at the
 * first failure and reports which module blocked the rest.
 */
export async function removeCollectionEntries(args: {
  entries: readonly InstalledSkill[];
  call: CollectionCall;
  signal: AbortSignal;
  onProgress: (progress: CollectionProgress) => void;
}): Promise<{ removed: number; removedIds: string[]; failed?: { title: string; message: string } }> {
  const { entries, call, signal, onProgress } = args;
  const removedIds: string[] = [];
  for (const [index, entry] of entries.entries()) {
    if (signal.aborted) throw new CollectionCancelledError();
    onProgress({
      index: index + 1,
      total: entries.length,
      title: entry.record.title,
      phase: 'remove',
    });
    try {
      await call('remove', { recordId: entry.record.id });
    } catch (error) {
      if (isCollectionCancelled(error)) throw error;
      // A call that failed because the user stopped the task is not a failure.
      if (signal.aborted) throw new CollectionCancelledError();
      return {
        removed: removedIds.length,
        removedIds,
        failed: {
          title: entry.record.title,
          message: error instanceof Error ? error.message : String(error),
        },
      };
    }
    removedIds.push(entry.record.id);
  }
  return { removed: removedIds.length, removedIds };
}
