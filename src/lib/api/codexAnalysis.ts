import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable } from './native';
import type { CodexHttpInfo, CodexLogConfig } from './codexLogs';

export interface AnalysisSource {
  id: string;
  role: string;
  format: string;
  name: string;
  path: string;
  size: number;
  walSize: number;
  modifiedAt: number;
  status: 'available' | 'missing' | 'error' | 'skipped';
  records: number;
  partial: boolean;
  note: string;
}
export interface EvidenceLocation {
  sourceId: string;
  sourceRole: string;
  path: string;
  position: string;
}
export interface AnalysisEvent {
  id: string;
  time: number;
  level: string;
  kind: string;
  target: string;
  threadId?: string;
  turnId?: string;
  model?: string;
  summary: string;
  http?: CodexHttpInfo;
  location: EvidenceLocation;
  mirrors: EvidenceLocation[];
}
export interface AnalysisSession {
  id: string;
  title: string;
  cwd: string;
  provider: string;
  model: string;
  origin: string;
  updatedAt: number;
  archived: boolean;
  requests: number;
  failures: number;
  retries: number;
  turnErrors: number;
  hosts: string[];
  rolloutPath: string;
  routeStatus: 'matches' | 'different' | 'unverified';
}
export interface AnalysisIssue {
  id: string;
  title: string;
  category: string;
  severity: string;
  count: number;
  firstAt: number;
  lastAt: number;
  advice: string;
  sample: string;
  eventIds: string[];
  threads: string[];
}
export interface Analysis {
  id: string;
  generatedAt: number;
  window: number;
  since: number;
  codexHome: string;
  config: CodexLogConfig;
  partial: boolean;
  mirrored: number;
  eventCount: number;
  sources: AnalysisSource[];
  totals: {
    requests: number;
    ok: number;
    failed: number;
    retries: number;
    turnErrors: number;
    warnings: number;
    errors: number;
    sessions: number;
  };
  routes: {
    address: string;
    requests: number;
    ok: number;
    failed: number;
    statuses: Record<string, number>;
    lastAt: number;
    sessionIds: string[];
  }[];
  sessions: AnalysisSession[];
  issues: AnalysisIssue[];
  notes: string[];
  recent: AnalysisEvent[];
}
export interface AnalysisFilter {
  kinds?: string[];
  levels?: string[];
  threadId?: string;
  search?: string;
  outcome?: 'ok' | 'failed';
}
export interface AnalysisPage {
  events: AnalysisEvent[];
  total: number;
  nextOffset?: number;
  snapshotId: string;
}
export interface AnalysisDetail {
  event: AnalysisEvent;
  message: string;
  span: string;
  note: string;
}
export interface SessionAnalysis {
  session: AnalysisSession;
  source: AnalysisSource;
  usage?: {
    input: number;
    cachedInput: number;
    output: number;
    reasoningOutput: number;
    total: number;
    contextWindow?: number;
    observedAt: number;
  };
  turns: { started: number; completed: number; aborted: number };
  toolCalls: number;
  events: AnalysisEvent[];
  eventDetails: Record<string, string>;
  hasMore: boolean;
  note: string;
}
async function call<T>(action: string, options: Record<string, unknown>): Promise<T> {
  if (!nativeAvailable) throw new Error('请在桌面版使用 Codex 日志分析。');
  try {
    return await invoke<T>('codex_logs', { request: { action, ...options } });
  } catch (error) {
    throw new Error(String(error));
  }
}
export const analyzeCodex = (window: number) => call<Analysis>('analyze', { window });
export const analysisEvents = (
  snapshotId: string,
  query: AnalysisFilter,
  sourceId?: string,
  offset = 0,
) => call<AnalysisPage>('analysis-events', { snapshotId, query, sourceId, offset });
export const analysisDetail = (snapshotId: string, eventId: string) =>
  call<AnalysisDetail>('analysis-detail', { snapshotId, eventId });
export const analyzeSession = (snapshotId: string, threadId: string) =>
  call<SessionAnalysis>('analysis-session', { snapshotId, threadId });
export const exportAnalysis = (snapshotId: string, threadId?: string) =>
  call<{ path: string }>('analysis-export', { snapshotId, threadId });
