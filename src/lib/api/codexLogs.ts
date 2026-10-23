import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable } from './native';

export interface CodexLogSource {
  path: string;
  name: string;
  size: number;
  walSize: number;
  modifiedAt: number;
  rows: number;
  oldest: number;
  newest: number;
  readable: boolean;
  error?: string;
}
export interface CodexProviderUrl {
  name: string;
  baseUrl: string;
}
export interface CodexLogConfig {
  provider: string;
  model: string;
  baseUrl: string;
  expectedHost: string;
  expectedScheme: string;
  providers: CodexProviderUrl[];
}
export interface CodexLogThreadMeta {
  id: string;
  title: string;
  provider: string;
  model: string;
}
export interface CodexLogSources {
  logs: CodexLogSource;
  state: CodexLogSource;
  config: CodexLogConfig;
  threads: CodexLogThreadMeta[];
  codexHome: string;
}
export interface CodexHttpInfo {
  method: string;
  url: string;
  scheme: string;
  host: string;
  path: string;
  status?: number;
  reason?: string;
  ok: boolean;
  timeout: boolean;
  connect: boolean;
  requestId?: string;
  clientRequestId?: string;
  server?: string;
  contentType?: string;
}
export type CodexEventKind =
  'request' | 'retry' | 'turn_error' | 'sse' | 'mcp' | 'tool' | 'session' | 'error' | 'other';
export interface CodexEvent {
  id: number;
  time: number;
  level: string;
  target: string;
  kind: CodexEventKind;
  threadId?: string;
  turnId?: string;
  pid?: number;
  model?: string;
  summary: string;
  http?: CodexHttpInfo;
}
export interface CodexStatusCount {
  status: number;
  count: number;
}
export interface CodexRouteStat {
  host: string;
  scheme: string;
  requests: number;
  ok: number;
  failed: number;
  statuses: CodexStatusCount[];
  lastAt: number;
  lastStatus?: number;
  configured: boolean;
  threads: number;
}
export interface CodexThreadStat {
  id: string;
  title?: string;
  provider?: string;
  model?: string;
  requests: number;
  failures: number;
  lastAt: number;
  hosts: string[];
  routeOk: boolean;
}
export interface CodexProcessStat {
  pid: number;
  events: number;
  requests: number;
  lastAt: number;
}
export interface CodexCluster {
  signature: string;
  title: string;
  count: number;
  level: string;
  target: string;
  firstAt: number;
  lastAt: number;
  sample: string;
  advice: string;
  benign: boolean;
  ids: number[];
}
export interface CodexTotals {
  requests: number;
  ok: number;
  failed: number;
  retries: number;
  turnErrors: number;
  warnings: number;
  errors: number;
  tools: number;
  threads: number;
}
export interface CodexVerdict {
  status: 'confirmed' | 'mismatch' | 'failing' | 'unverified';
  title: string;
  message: string;
  evidence: string[];
}
export interface CodexOverview {
  generatedAt: number;
  window: number;
  source: CodexLogSource;
  state: CodexLogSource;
  config: CodexLogConfig;
  totals: CodexTotals;
  routes: CodexRouteStat[];
  threads: CodexThreadStat[];
  processes: CodexProcessStat[];
  errors: CodexCluster[];
  recent: CodexEvent[];
  verdict: CodexVerdict;
  notes: string[];
}
export interface CodexEventQuery {
  kinds?: CodexEventKind[];
  levels?: string[];
  threadId?: string;
  host?: string;
  search?: string;
  since?: number;
  until?: number;
  afterId?: number;
  beforeId?: number;
  limit?: number;
  outcome?: 'ok' | 'failed';
}
export interface CodexEventPage {
  events: CodexEvent[];
  hasMore: boolean;
  nextBefore?: number;
  nextAfter?: number;
  scanned: number;
  missing?: boolean;
}
export interface CodexLogDetail {
  id: number;
  time: number;
  level: string;
  target: string;
  threadId?: string;
  pid?: number;
  span: string;
  message: string;
}

async function call<T>(action: string, options: Record<string, unknown> = {}): Promise<T> {
  if (!nativeAvailable) throw new Error('请在桌面版使用 Codex 日志分析。');
  try {
    return await invoke<T>('codex_logs', { request: { action, ...options } });
  } catch (error) {
    throw new Error(String(error));
  }
}

export function codexLogSources(): Promise<CodexLogSources> {
  return call('sources');
}
export function codexLogOverview(windowSeconds: number): Promise<CodexOverview> {
  return call('overview', { window: windowSeconds });
}
export function codexLogEvents(query: CodexEventQuery): Promise<CodexEventPage> {
  return call('events', { query });
}
export function codexLogDetail(id: number): Promise<CodexLogDetail> {
  return call('detail', { id });
}
export function codexLogExport(windowSeconds: number): Promise<{ path: string }> {
  return call('export', { window: windowSeconds });
}
