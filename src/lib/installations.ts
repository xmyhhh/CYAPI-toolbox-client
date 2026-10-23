import type { NativeRecord, NativeStatus } from './api/native';

type Installation = NativeStatus['installed'][number];
type Target = NativeRecord['target'];

export function visibleInWorkspace(entry: Installation, workspace: Target): boolean {
  return entry.visibleTargets?.includes(workspace) ?? entry.record.target === workspace;
}

export function canManageInstallation(entry: Installation, workspace: Target): boolean {
  return entry.managed !== false && entry.record.target === workspace;
}
