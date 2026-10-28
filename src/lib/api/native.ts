import { invoke, isTauri } from '@tauri-apps/api/core';
import { CodexSaveConflict, codexSaveConflictMessage } from '../codex-save';
import type { IntegrationGuide, SoftwareCompatibility } from '../../../shared/integration-guide';
export const SERVER_URL = import.meta.env.VITE_CYAPI_SERVER_URL || 'https://api.cyapi.cyou';
export type ConfigValue = string | number | boolean | null;
export interface ConfigField {
  id: string;
  label: string;
  type: 'text' | 'url' | 'secret' | 'select' | 'number' | 'boolean';
  required?: boolean;
  default?: ConfigValue;
  placeholder?: string;
  options?: string[];
  minimum?: number;
  maximum?: number;
}
export interface ConfigFlow {
  schema: 1;
  revision: string;
  steps: { id: string; title: string; fields: ConfigField[] }[];
  files: {
    root: 'home' | 'appData' | 'localAppData' | 'skill';
    path: string;
    format: 'json' | 'yaml' | 'dotenv';
    bindings: { key: string[]; field: string }[];
  }[];
}
export interface ConfigurationView {
  flow: ConfigFlow;
  flowRevision: string;
  configurationRevision: string;
  values: Record<string, ConfigValue>;
  configuredSecrets: Record<string, boolean>;
  files: { path: string; format: string; exists: boolean }[];
}
export interface RuntimeInfo {
  kind: 'node' | 'python';
  version: string;
  path: string;
  source: 'codex' | 'system' | 'managed';
}
export interface Requirement {
  kind: 'node' | 'python';
  minimum: string;
}
export interface ServerSkill {
  id: string;
  title: string;
  description: string;
  version: string;
  revision: string;
  sourceUrl: string;
  sha256: string;
  size: number;
  fileCount: number;
  packagePath: string;
  requirements: Requirement[];
  markdown: string;
  category?: string;
  setupNotes?: string[];
  license?: string;
  distribution?: 'bundled' | 'source-only';
  distributionReason?: string | null;
  recommended?: boolean;
  configuration?: ConfigFlow | null;
  sharedDependencies?: string[];
}
/** Server-owned suite; items reference independently installable catalog Skills. */
export interface SkillGroup {
  id: string;
  title: string;
  description: string;
  sourceUrl: string;
  items: { skillId: string }[];
}
export interface NativeRecord {
  id: string;
  skillId: string;
  title: string;
  version: string;
  revision: string;
  target: 'codex' | 'claude' | 'opencode';
  scope: 'global' | 'project';
  path: string;
  installedAt: number;
  runtimes: RuntimeInfo[];
  packageSha256: string;
  configuration?: ConfigFlow | null;
}
export interface NativeStatus {
  settings: { serverUrl: string };
  online: boolean;
  message?: string;
  generatedAt?: string;
  /** Optional for compatibility with servers and caches predating groups. */
  skillGroups?: SkillGroup[];
  skills: {
    skill: ServerSkill;
    downloaded: boolean;
    offlineReady: boolean;
    installed?: boolean;
    installedPaths?: string[];
    /** The installed version differs from the one the catalog publishes. */
    updateAvailable?: boolean;
  }[];
  runtimes: RuntimeInfo[];
  installed: {
    record: NativeRecord;
    exists: boolean;
    managed?: boolean;
    /** Explicit backend permission; older clients only support managed uninstall. */
    removable?: boolean;
    removalBlockedReason?: string | null;
    visibleTargets?: NativeRecord['target'][];
  }[];
  dataPath: string;
  homePath: string;
}
export interface NativePlan {
  path: string;
  conflict: boolean;
  installed?: boolean;
  offlineReady: boolean;
  runtimes: {
    requirement: Requirement;
    reuse?: RuntimeInfo;
    fallback?: { version: string; size: number };
    error?: string;
  }[];
}
/** Per-model values declared into the generated Codex model catalog. */
export interface CodexModelSetting {
  contextWindow?: number | null;
  reasoningLevels?: string[] | null;
  defaultLevel?: string | null;
}
export interface CodexConfiguration {
  /** Raw explicit override; not inferred model capacity. */
  contextWindow?: unknown;
  configPath: string;
  authPath: string;
  revision: string;
  /** Tracks the settings edited here; revision still protects full-file restores. */
  saveRevision?: string;
  /** Allows field-wise rebase only while protected provider/auth settings match. */
  rebaseGuard?: string;
  provider: string;
  baseUrl: string;
  model: string;
  models: string[];
  reasoningEffort: string;
  modelSettings?: Record<string, CodexModelSetting>;
  apiKeyConfigured: boolean;
  apiKey?: string;
  authMode: string;
  canRestore: boolean;
}
export interface ClaudeConfiguration {
  /** Raw env.CLAUDE_CODE_MAX_CONTEXT_TOKENS; applicability depends on Claude Code. */
  contextWindow?: unknown;
  modelEfforts?: Record<string, string>;
  modelEffortKeys?: Record<string, string>;
  effortOverrides?: { environment: string; global: string };
  configPath: string;
  revision: string;
  baseUrl: string;
  model: string;
  models: string[];
  reasoningEffort: string;
  authTokenConfigured: boolean;
  authToken?: string;
  apiKeyConfigured: boolean;
  authMode: string;
  settingsExists: boolean;
  cliInstalled: boolean;
  cliPath: string;
  canRestore: boolean;
}
export interface OpenCodeConfiguration {
  modelReasoningLevels?: Record<string, unknown>;
  modelReasoningLevelConflicts?: string[];
  reasoningLevelsEditable?: boolean;
  modelContexts?: Record<string, unknown>;
  modelContextConflicts?: string[];
  modelEfforts?: Record<string, string>;
  modelEffortConflicts?: string[];
  configPath: string;
  revision: string;
  baseUrl: string;
  model: string;
  models: string[];
  reasoningEffort: string;
  apiKeyConfigured: boolean;
  apiKey?: string;
  authMode: string;
  settingsExists: boolean;
  cliInstalled: boolean;
  cliPath: string;
  canRestore: boolean;
}
export interface IntegrationField {
  id: string;
  label: string;
  type: 'text' | 'secret' | 'number' | 'boolean';
  default: ConfigValue;
  placeholder: string;
}
export interface IntegrationAction {
  id: string;
  label: string;
  tool: string;
  args: Record<string, ConfigValue>;
  fields: IntegrationField[];
  secondary: boolean;
  result?: {
    message?: string;
    listPath?: string[];
    columns?: { label: string; path: string[] }[];
    path?: string[];
  };
}
export interface IntegrationComponent {
  version: string;
  platform: string;
  path: string;
  sha256: string;
  size: number;
  licenseText?: string;
}
export interface IntegrationDescriptor {
  id: string;
  name: string;
  minimumVersion: string;
  downloadUrl: string;
  capabilities: string[];
  steps: string[];
  skillId?: string;
  relatedMcpId?: string;
  guide?: IntegrationGuide;
  compatibility?: SoftwareCompatibility;
  workspaces?: ('codex' | 'claude' | 'opencode')[];
  replacesMcp?: string;
  component?: IntegrationComponent;
  /** The component builds of every platform the service publishes. */
  components?: IntegrationComponent[];
  mcp?: { name: string; args: string[]; timeoutSeconds: number };
  statusTool?: string;
  actions: IntegrationAction[];
}
export interface IntegrationManifest {
  schema: 2 | 3 | 4;
  integrations: IntegrationDescriptor[];
}
export interface IntegrationRequirement {
  kind: string;
  id: string;
  optional: boolean;
  satisfied: boolean;
}
export interface IntegrationEntry {
  id: string;
  downloaded: boolean;
  registered: boolean;
  managed: boolean;
  conflict: boolean;
  updateAvailable: boolean;
  /** The revision of the MCP entry this integration writes to. */
  mcpRevision?: string;
  state?: McpEntryStatus['state'];
  canInstall?: boolean;
  canAdopt?: boolean;
  compatible?: boolean;
  enabled?: boolean;
  configPath?: string;
  software?: {
    found: boolean | null;
    version?: string | null;
    source?: string | null;
    state: 'unknown' | 'incompatible' | 'tested' | 'untested';
    message: string;
  };
  relatedMcp?: { id: string; configured: boolean; runtimeReady?: boolean; state?: string };
  /** Plugin files are prepared; enabling and starting the in-app service is separate. */
  addonReady?: boolean;
  service?: IntegrationServiceStatus;
  readiness?: {
    state: 'not-installed' | 'needs-attention' | 'configured' | 'ready';
    checks: {
      id: string;
      title: string;
      state: 'ready' | 'blocked' | 'unverified';
      message: string;
    }[];
  };
  /** The integration is installed and its prerequisites are satisfied. */
  usable?: boolean;
  requires?: IntegrationRequirement[];
  requirementsMet?: boolean;
  health: {
    ready: boolean;
    message: string;
    rows: { label: string; value: string }[];
    values?: Record<string, ConfigValue>;
  };
}

export interface IntegrationServiceStatus {
  state: 'connected' | 'waiting' | 'timeout' | 'invalid-response' | 'unverified';
  address?: string | null;
  message: string;
  checkedAt: number;
}
export interface IntegrationServicesView {
  workspace: 'codex' | 'claude' | 'opencode';
  entries: { id: string; mcpRevision?: string | null; service: IntegrationServiceStatus }[];
}
export interface IntegrationView {
  manifest: IntegrationManifest;
  source: 'cached';
  configPath?: string;
  revision: string;
  /** The platform identifier of the machine the client runs on. */
  platform?: string;
  workspace?: 'codex' | 'claude' | 'opencode';
  entries: IntegrationEntry[];
}
export interface McpTransportStdio {
  type: 'stdio';
  command: string;
  args: string[];
  startupTimeoutSeconds: number;
  toolTimeoutSeconds: number;
}
export interface McpTransportHttp {
  type: 'http';
  url: string;
  tokenSource?: string;
  /** The agent reads the credential from this environment variable. */
  bearerTokenEnvVar?: string;
}
export type McpTransport = McpTransportStdio | McpTransportHttp;
export interface McpDescriptor {
  id: string;
  configKey?: string;
  name: string;
  version: string;
  description: string;
  homepage: string;
  capabilities: string[];
  steps: string[];
  installCommand: string;
  /** The install command of every platform the service publishes. */
  installCommands?: Record<string, string>;
  transport: McpTransport;
  workspaces: ('codex' | 'claude' | 'opencode')[];
  minClientVersion?: string;
}
export interface McpCatalog {
  schema: 1 | 2;
  generatedAt: string;
  servers: McpDescriptor[];
}
export interface McpEntryStatus {
  id: string;
  /** The revision of this entry, the only part an action may write. */
  revision: string;
  /** One mutually exclusive state per entry. */
  state: 'absent' | 'managed' | 'changed' | 'stale' | 'adoptable' | 'conflict' | 'incompatible';
  installed: boolean;
  managed: boolean;
  modified: boolean;
  external: boolean;
  recognized: boolean;
  adoptable: boolean;
  conflict: boolean;
  compatible: boolean;
  conflictReason?: string | null;
  tokenAvailable: boolean;
  canInstall: boolean;
  canAdopt: boolean;
  canRemove: boolean;
  canEnable: boolean;
  canDisable: boolean;
  /** The entry was defined on this machine instead of by the server. */
  local?: boolean;
  transport: 'stdio' | 'http';
  enabled: boolean;
}
export interface McpLocalEntry {
  name: string;
  /** The revision of this entry, the only part an action may write. */
  revision: string;
  transport: string;
  target: string;
  enabled: boolean;
  managed: boolean;
  /** Runtime-owned, duplicate or unrecognized entries cannot be edited here. */
  readOnlyReason: string | null;
  catalogId: string | null;
  duplicate?: boolean;
}
export interface McpView {
  catalog: McpCatalog;
  workspace: 'codex' | 'claude' | 'opencode';
  configPath: string;
  revision: string;
  source: 'cached' | 'local' | 'browser';
  catalogError?: string | null;
  entries: McpEntryStatus[];
  local: McpLocalEntry[];
}
export const nativeAvailable = isTauri();
// 后台读取（状态、预览、更新检查等）失败只在日志中记录，不自动弹出日志窗口。
const quietActions = [
  'status',
  'catalog-read',
  'preview',
  'codex-read',
  'claude-read',
  'opencode-read',
  'models-list',
  'configuration-read',
  'mcp-view',
  'mcp-catalog-read',
  'mcp-refresh',
  'integrations-read',
  'integrations-refresh',
  'integration-services-read',
  'update-check',
];
export async function nativeCall<T>(
  action: string,
  options: Record<string, unknown> = {},
): Promise<T> {
  if (!nativeAvailable) throw new Error('请在桌面客户端中执行此操作；网页预览只展示服务端列表。');
  const operationId =
    typeof options.operationId === 'string' ? options.operationId : crypto.randomUUID();
  const quiet = quietActions.includes(action);
  if (!quiet) {
    window.dispatchEvent(new CustomEvent('toolbox-operation', { detail: { operationId } }));
  }
  try {
    return await invoke<T>('toolbox', { request: { action, ...options, operationId } });
  } catch (error) {
    const conflict = codexSaveConflictMessage(action, String(error));
    if (conflict) throw new CodexSaveConflict(conflict);
    if (!quiet) {
      window.dispatchEvent(
        new CustomEvent('toolbox-operation-failed', { detail: { operationId } }),
      );
    }
    throw new Error(String(error));
  }
}
/**
 * Stop a running installation. The command does not wait for the service, so
 * it reaches the download that is in flight.
 */
export async function cancelNativeOperation() {
  if (!nativeAvailable) return;
  try {
    await invoke('toolbox_cancel');
  } catch {
    // The task stops at the next step either way.
  }
}

export async function browserCatalog(): Promise<NativeStatus> {
  const response = await fetch(`${SERVER_URL}/api/catalog`);
  if (!response.ok) throw new Error('服务暂不可用，请稍后重试。');
  const catalog = (await response.json()) as {
    skills: ServerSkill[];
    generatedAt: string;
    skillGroups?: SkillGroup[];
  };
  return {
    settings: { serverUrl: SERVER_URL },
    online: true,
    generatedAt: catalog.generatedAt,
    skillGroups: catalog.skillGroups ?? [],
    skills: catalog.skills.map((skill) => ({ skill, downloaded: false, offlineReady: false })),
    runtimes: [],
    installed: [],
    dataPath: '请在桌面版查看',
    homePath: '',
  };
}
