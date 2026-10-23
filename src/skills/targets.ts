import type { Settings, Skill, TargetAdapter } from './types';

export const targets: TargetAdapter[] = [
  {
    id: 'codex',
    name: 'Codex',
    subtitle: 'OpenAI',
    globalDirectory: '~/.codex/skills',
    projectDirectory: '.agents/skills',
  },
  {
    id: 'claude',
    name: 'Claude Code',
    subtitle: 'Anthropic',
    globalDirectory: '~/.claude/skills',
    projectDirectory: '.claude/skills',
  },
  {
    id: 'opencode',
    name: 'OpenCode',
    subtitle: 'OpenCode',
    globalDirectory: '~/.config/opencode/skills',
    projectDirectory: '.opencode/skills',
  },
];

export function installationPath(skill: Skill, settings: Settings): string {
  // Also validate at the service boundary, before creating a target path.
  if (!/^[a-z0-9]+(?:-[a-z0-9]+)*$/.test(skill.name)) throw new Error('Skill 名称不合法。');
  const target = targets.find((item) => item.id === settings.defaultTarget);
  if (!target) throw new Error('请选择有效的 Agent。');
  if (settings.scope === 'global') return `${target.globalDirectory}/${skill.name}`;
  const project = settings.projectPath.trim().replace(/[\\/]+$/, '');
  if (!project || !/^(?:[a-zA-Z]:[\\/]|\/[^/]|\\\\[^\\]+\\[^\\]+)/.test(project)) {
    throw new Error('请填写项目的绝对路径，例如 C:\\Projects\\my-app。');
  }
  if (project.split(/[\\/]/).some((part) => part === '..' || part === '.')) {
    throw new Error('项目路径不能包含 . 或 .. 片段。');
  }
  const separator = project.includes('\\') ? '\\' : '/';
  const directory = target.projectDirectory.replaceAll('/', separator);
  return `${project}${separator}${directory}${separator}${skill.name}`;
}
