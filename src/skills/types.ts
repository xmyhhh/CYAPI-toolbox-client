export type TargetId = 'codex' | 'claude' | 'opencode';
export type Scope = 'global' | 'project';
export type Category = '开发效率' | '设计创作' | '文档处理' | '工作流程';
export type SkillIcon = 'terminal' | 'palette' | 'file' | 'git' | 'search' | 'blocks';

export interface Skill {
  id: string;
  name: string;
  title: string;
  description: string;
  category: Category;
  icon: SkillIcon;
  author: string;
  version: string;
  source: 'example' | 'local';
  fileCount: number;
  markdown: string;
}

export interface Settings {
  defaultTarget: TargetId;
  scope: Scope;
  projectPath: string;
}

export interface InstallRecord {
  id: string;
  skill: Skill;
  target: TargetId;
  scope: Scope;
  path: string;
  installedAt: string;
}

export interface Activity {
  id: string;
  action: 'install' | 'replace' | 'uninstall' | 'import';
  name: string;
  target?: TargetId;
  path?: string;
  at: string;
}

export interface Workspace {
  schema: 1;
  imported: Skill[];
  installed: InstallRecord[];
  activities: Activity[];
  settings: Settings;
}

export interface InstallPlan {
  skill: Skill;
  target: TargetId;
  scope: Scope;
  path: string;
  conflict?: InstallRecord;
}

export interface TargetAdapter {
  id: TargetId;
  name: string;
  subtitle: string;
  globalDirectory: string;
  projectDirectory: string;
}

// The UI depends on this contract. A future Tauri implementation can invoke Rust here.
export interface InstallerService {
  mode: 'demo' | 'native';
  preview(skill: Skill, settings: Settings, installed: InstallRecord[]): InstallPlan;
  install(
    plan: InstallPlan,
    replace: boolean,
    onProgress: (step: number) => void,
  ): Promise<InstallRecord>;
}
