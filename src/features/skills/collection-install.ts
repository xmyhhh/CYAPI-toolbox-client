import type { NativePlan, NativeRecord } from '../../lib/api/native.ts';
import { pathKey } from './collections.ts';
import type { CatalogSkill, InstalledSkill } from './collections.ts';

export interface CollectionLocation {
  target: NativeRecord['target'];
  scope: NativeRecord['scope'];
  projectPath: string;
}
export type CollectionCall = <T>(action: string, options?: Record<string, unknown>) => Promise<T>;
export interface CollectionStep {
  entry: CatalogSkill;
  plan?: NativePlan;
  action: 'install' | 'update' | 'keep' | 'blocked';
  note: string;
}

/** The stage one module is in, so the UI can say what a long wait is about. */
export type CollectionPhase = 'check' | 'download' | 'install' | 'update' | 'remove';
export interface CollectionProgress {
  index: number;
  total: number;
  title: string;
  phase: CollectionPhase;
}
export const collectionPhaseLabels: Record<CollectionPhase, string> = {
  check: '正在检查安装位置',
  download: '正在下载离线包',
  install: '正在写入文件',
  update: '正在更新文件',
  remove: '正在移至备份',
};

/** Raised when the user stops a run. It is a decision, not an install failure. */
export class CollectionCancelledError extends Error {
  constructor() {
    super('已取消');
    this.name = 'CollectionCancelledError';
  }
}
export function isCollectionCancelled(error: unknown) {
  return error instanceof CollectionCancelledError;
}

function runtimeProblem(plan: NativePlan) {
  const missing = plan.runtimes.find((runtime) => !runtime.reuse && !runtime.fallback);
  return missing ? missing.error || '缺少兼容运行环境，请先准备环境' : undefined;
}

function dependencyOrder(entries: readonly CatalogSkill[]) {
  const byId = new Map(entries.map((entry) => [entry.skill.id, entry]));
  const visited = new Set<string>();
  const visiting = new Set<string>();
  const ordered: CatalogSkill[] = [];
  function visit(entry: CatalogSkill) {
    if (visiting.has(entry.skill.id)) throw new Error('套件中存在循环依赖');
    if (visited.has(entry.skill.id)) return;
    visiting.add(entry.skill.id);
    for (const id of entry.skill.sharedDependencies ?? []) {
      const dependency = byId.get(id);
      if (dependency) visit(dependency);
    }
    visiting.delete(entry.skill.id);
    visited.add(entry.skill.id);
    ordered.push(entry);
  }
  entries.forEach(visit);
  return ordered;
}

export async function previewCollection(
  entries: readonly CatalogSkill[],
  installed: readonly InstalledSkill[],
  location: CollectionLocation,
  call: CollectionCall,
): Promise<CollectionStep[]> {
  return Promise.all(
    dependencyOrder(entries).map(async (entry): Promise<CollectionStep> => {
      const step = { entry, action: 'blocked' as const, note: '' };
      if (entry.skill.distribution === 'source-only')
        return { ...step, action: 'keep', note: '需从官方获取' };
      try {
        const plan = await call<NativePlan>('preview', { skillId: entry.skill.id, ...location });
        if (plan.installed) {
          const existing = installed.find(
            ({ record }) => pathKey(record.path) === pathKey(plan.path),
          );
          if (existing && !existing.exists)
            return { ...step, plan, note: '安装文件已变化，请先检查' };
          if (
            !existing ||
            existing.managed === false ||
            existing.record.target !== location.target
          ) {
            return { ...step, plan, action: 'keep', note: '保留本机已有版本' };
          }
          if (existing.record.packageSha256 === entry.skill.sha256)
            return { ...step, plan, action: 'keep', note: '已是当前版本' };
          const problem = runtimeProblem(plan);
          if (problem) return { ...step, plan, note: problem };
          return { ...step, plan, action: 'update', note: '更新并备份旧版本' };
        }
        if (plan.conflict) return { ...step, plan, note: '目标目录已存在，请更换安装位置' };
        const problem = runtimeProblem(plan);
        if (problem) return { ...step, plan, note: problem };
        return { ...step, plan, action: 'install', note: '安装' };
      } catch (error) {
        return { ...step, note: error instanceof Error ? error.message : String(error) };
      }
    }),
  );
}

export interface CollectionResult {
  installed: number;
  updated: number;
  kept: number;
  failed?: { title: string; message: string };
}

export async function installCollection(
  steps: readonly CollectionStep[],
  location: CollectionLocation,
  call: CollectionCall,
  onProgress: (progress: CollectionProgress) => void,
  signal?: AbortSignal,
): Promise<CollectionResult> {
  if (steps.some((step) => step.action === 'blocked')) throw new Error('请先处理套件中的安装问题');
  const result: CollectionResult = { installed: 0, updated: 0, kept: 0 };
  for (const [index, step] of steps.entries()) {
    if (signal?.aborted) throw new CollectionCancelledError();
    if (step.action === 'keep') {
      result.kept++;
      continue;
    }
    const options = { skillId: step.entry.skill.id, ...location };
    const report = (phase: CollectionPhase) =>
      onProgress({
        index: index + 1,
        total: steps.length,
        title: step.entry.skill.title,
        phase,
      });
    report('check');
    try {
      // Recheck before writing: a previous member may have installed a shared
      // dependency, or another process may have created the destination.
      const current = await call<NativePlan>('preview', options);
      if (signal?.aborted) throw new CollectionCancelledError();
      if (step.action === 'install' && current.installed) {
        result.kept++;
        continue;
      }
      if (step.action === 'install' && current.conflict) throw new Error('目标目录已存在');
      const problem = runtimeProblem(current);
      if (problem) throw new Error(problem);
      if (!current.offlineReady) {
        report('download');
        await call('download', { skillId: step.entry.skill.id });
        if (signal?.aborted) throw new CollectionCancelledError();
      }
      report(step.action === 'update' ? 'update' : 'install');
      await call<NativeRecord>(step.action === 'update' ? 'update-skill' : 'install', options);
      if (step.action === 'update') result.updated++;
      else result.installed++;
    } catch (error) {
      if (isCollectionCancelled(error)) throw error;
      // A call that failed because the user stopped the task is not a failure.
      if (signal?.aborted) throw new CollectionCancelledError();
      result.failed = {
        title: step.entry.skill.title,
        message: error instanceof Error ? error.message : String(error),
      };
      break;
    }
  }
  return result;
}
