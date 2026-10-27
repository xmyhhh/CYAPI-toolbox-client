// Business workbenches are independent of the selected Agent/tool.
export type Workbench = 'create' | 'tools';
export type ToolPage =
  | 'catalog'
  | 'installed'
  | 'codex'
  | 'mcp'
  | 'diagnostics'
  | 'usage'
  | 'environment'
  | 'integrations';
export type CreationSection = 'library' | 'drafts' | 'queue' | 'results';
export type WorkbenchNavigation = {
  workbench: Workbench;
  toolPage: ToolPage;
  creationSection: CreationSection;
};
export const WORKBENCH_KEY = 'cyapi.workbenches.v1';
export const creationSections = [
  ['library', '工作流库'],
  ['drafts', '草稿'],
  ['queue', '任务队列'],
  ['results', '结果'],
] as const;
const toolPages: readonly string[] = [
  'catalog',
  'installed',
  'codex',
  'mcp',
  'diagnostics',
  'usage',
  'environment',
  'integrations',
];
export const isToolPage = (value: unknown): value is ToolPage =>
  typeof value === 'string' && toolPages.includes(value);
export const isCreationSection = (value: unknown): value is CreationSection =>
  creationSections.some(([id]) => id === value);

export function parseWorkbenchNavigation(saved: string | null): WorkbenchNavigation {
  const fallback: WorkbenchNavigation = {
    workbench: 'create',
    toolPage: 'codex',
    creationSection: 'library',
  };
  try {
    const value: unknown = JSON.parse(saved || 'null');
    if (!value || typeof value !== 'object') return fallback;
    const item = value as Record<string, unknown>;
    return {
      workbench: item.workbench === 'tools' ? 'tools' : 'create',
      toolPage: isToolPage(item.toolPage) ? item.toolPage : 'codex',
      creationSection: isCreationSection(item.creationSection) ? item.creationSection : 'library',
    };
  } catch {
    return fallback;
  }
}

export function loadWorkbenchNavigation(): WorkbenchNavigation {
  try {
    return parseWorkbenchNavigation(localStorage.getItem(WORKBENCH_KEY));
  } catch {
    return parseWorkbenchNavigation(null);
  }
}
