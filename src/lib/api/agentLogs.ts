import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable } from './native';

export type LogWorkspace = 'claude' | 'opencode';
export interface LogTokens {
  input: number;
  output: number;
  cacheRead: number;
  cacheWrite: number;
  cacheWrite1h: number;
  reasoning: number;
}
export interface LogStats {
  responses: number;
  responseErrors: number;
  apiErrors: number;
  retries: number;
  toolCalls: number;
  toolErrors: number;
  compactions: number;
  userTurns: number;
  errors: number;
  warnings: number;
  usageRecords: number;
  tokens: LogTokens;
}
export interface LogSource {
  id: string;
  role: string;
  format: string;
  path: string;
  size: number;
  walSize: number;
  records: number;
  status: 'available' | 'missing' | 'error';
  partial: boolean;
  note: string;
}
export interface LogLocation {
  sourceId: string;
  path: string;
  position: string;
}
export interface AgentLogEvent {
  id: string;
  time: number;
  level: string;
  kind: string;
  target: string;
  sessionId?: string;
  messageId?: string;
  model: string;
  provider: string;
  summary: string;
  failed: boolean;
  status?: number;
  requestId?: string;
  durationMs?: number;
  usage?: LogTokens;
  location: LogLocation;
  mirrors: LogLocation[];
}
export interface LogSession {
  id: string;
  title: string;
  project: string;
  model: string;
  provider: string;
  agent: string;
  parentId?: string;
  forkedFrom?: string;
  updatedAt: number;
  stats: LogStats;
}
export interface LogIssue {
  id: string;
  category: string;
  title: string;
  advice: string;
  count: number;
  firstAt: number;
  lastAt: number;
  eventIds: string[];
  sessionIds: string[];
}
export interface AgentLogSnapshot {
  id: string;
  workspace: LogWorkspace;
  home: string;
  generatedAt: number;
  since: number;
  window: number;
  partial: boolean;
  mirrored: number;
  eventCount: number;
  totals: LogStats;
  sessions: LogSession[];
  models: { model: string; provider: string; stats: LogStats }[];
  sources: LogSource[];
  issues: LogIssue[];
  notes: string[];
  recent: AgentLogEvent[];
}
export interface AgentLogQuery {
  search?: string;
  sessionId?: string;
  sourceId?: string;
  kind?: string;
  level?: string;
  failed?: boolean;
}
export interface AgentLogPage {
  snapshotId: string;
  events: AgentLogEvent[];
  total: number;
  nextOffset?: number;
}
export interface AgentLogDetail {
  event: AgentLogEvent;
  evidence: unknown;
  note: string;
}
export interface LogSessionDetail {
  session: LogSession;
  events: AgentLogEvent[];
  children: LogSession[];
  hasMore: boolean;
  note: string;
}
async function call<T>(
  workspace: LogWorkspace,
  action: string,
  options: Record<string, unknown>,
): Promise<T> {
  if (!nativeAvailable) throw new Error('请在桌面版使用日志分析。');
  try {
    return await invoke<T>('agent_logs', { request: { workspace, action, ...options } });
  } catch (error) {
    throw new Error(String(error));
  }
}
export const analyzeAgentLogs = (workspace: LogWorkspace, window: number) =>
  call<AgentLogSnapshot>(workspace, 'analyze', { window });
export const agentLogEvents = (
  workspace: LogWorkspace,
  snapshotId: string,
  query: AgentLogQuery,
  offset = 0,
) => call<AgentLogPage>(workspace, 'events', { snapshotId, query, offset });
export const agentLogDetail = (workspace: LogWorkspace, snapshotId: string, eventId: string) =>
  call<AgentLogDetail>(workspace, 'detail', { snapshotId, eventId });
export const agentLogSession = (workspace: LogWorkspace, snapshotId: string, sessionId: string) =>
  call<LogSessionDetail>(workspace, 'session', { snapshotId, sessionId });
export const exportAgentLogs = (workspace: LogWorkspace, snapshotId: string, sessionId?: string) =>
  call<{ path: string }>(workspace, 'export', { snapshotId, sessionId });

export interface AgentLogClient {
  available: boolean;
  analyze: typeof analyzeAgentLogs;
  events: typeof agentLogEvents;
  detail: typeof agentLogDetail;
  session: typeof agentLogSession;
  export: typeof exportAgentLogs;
}
export const localAgentLogClient: AgentLogClient = {
  available: nativeAvailable,
  analyze: analyzeAgentLogs,
  events: agentLogEvents,
  detail: agentLogDetail,
  session: agentLogSession,
  export: exportAgentLogs,
};
