import { installationPath } from '../../skills/targets.ts';
import type { InstallerService } from '../../skills/types.ts';

const wait = (duration: number) => new Promise<void>((resolve) => setTimeout(resolve, duration));

// Frontend simulation only. Replace this adapter with Tauri invoke() in phase two.
export const installer: InstallerService = {
  mode: 'demo',
  preview(skill, settings, installed) {
    const path = installationPath(skill, settings);
    const conflict = installed.find(
      (record) => record.target === settings.defaultTarget && record.path === path,
    );
    return { skill, target: settings.defaultTarget, scope: settings.scope, path, conflict };
  },
  async install(plan, replace, onProgress) {
    if (plan.conflict && !replace) throw new Error('目标已存在，请先确认替换模拟记录。');
    for (let step = 0; step < 4; step += 1) {
      onProgress(step);
      await wait(330);
    }
    onProgress(4);
    return {
      id: crypto.randomUUID(),
      skill: plan.skill,
      target: plan.target,
      scope: plan.scope,
      path: plan.path,
      installedAt: new Date().toISOString(),
    };
  },
};
