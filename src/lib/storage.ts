import type { Activity, Skill, Workspace } from '../skills/types';

const KEY = 'cyapi-toolbox.workspace.v1';
export const emptyWorkspace = (): Workspace => ({
  schema: 1,
  imported: [],
  installed: [],
  activities: [],
  settings: { defaultTarget: 'codex', scope: 'global', projectPath: '' },
});

function isSkill(value: unknown): value is Skill {
  if (!value || typeof value !== 'object') return false;
  const skill = value as Skill;
  return (
    ['id', 'name', 'title', 'description', 'author', 'version', 'markdown'].every(
      (key) => typeof (value as Record<string, unknown>)[key] === 'string',
    ) &&
    /^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(skill.name) &&
    ['example', 'local'].includes(skill.source) &&
    ['开发效率', '设计创作', '文档处理', '工作流程'].includes(skill.category) &&
    ['terminal', 'palette', 'file', 'git', 'search', 'blocks'].includes(skill.icon) &&
    Number.isInteger(skill.fileCount) &&
    skill.fileCount > 0
  );
}

export function loadWorkspace(): { workspace: Workspace; warning?: string } {
  try {
    const raw = localStorage.getItem(KEY);
    if (!raw) return { workspace: emptyWorkspace() };
    const value = JSON.parse(raw) as Workspace;
    if (
      value.schema !== 1 ||
      !Array.isArray(value.imported) ||
      !value.imported.every(isSkill) ||
      !Array.isArray(value.installed) ||
      !value.installed.every(
        (record) =>
          isSkill(record.skill) &&
          typeof record.id === 'string' &&
          typeof record.path === 'string' &&
          ['codex', 'claude', 'opencode'].includes(record.target) &&
          ['global', 'project'].includes(record.scope) &&
          Number.isFinite(Date.parse(record.installedAt)),
      ) ||
      !Array.isArray(value.activities) ||
      !value.activities.every(
        (activity) =>
          typeof activity.id === 'string' &&
          typeof activity.name === 'string' &&
          ['install', 'replace', 'uninstall', 'import'].includes(activity.action) &&
          Number.isFinite(Date.parse(activity.at)),
      ) ||
      !value.settings ||
      !['codex', 'claude', 'opencode'].includes(value.settings.defaultTarget) ||
      !['global', 'project'].includes(value.settings.scope) ||
      typeof value.settings.projectPath !== 'string'
    ) {
      throw new Error('Invalid workspace');
    }
    return { workspace: value };
  } catch {
    return { workspace: emptyWorkspace(), warning: '无法读取本机演示数据，已载入空白工作台。' };
  }
}

// Write before committing React state, so quota failures cannot look like success.
export function saveWorkspace(workspace: Workspace): void {
  try {
    localStorage.setItem(KEY, JSON.stringify(workspace));
  } catch {
    throw new Error('本机存储空间不足或不可用，操作未保存。请减少导入内容后重试。');
  }
}

export function newActivity(
  action: Activity['action'],
  name: string,
  extra: Partial<Activity> = {},
): Activity {
  return { ...extra, id: crypto.randomUUID(), action, name, at: new Date().toISOString() };
}
