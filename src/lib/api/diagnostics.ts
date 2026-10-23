import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable } from './native';

export interface DiagnosticStep {
  name: string;
  status: 'pass' | 'fail';
  message: string;
  osErrorCode?: number;
}
export interface DiagnosticCheck {
  id: string;
  title: string;
  status: 'pass' | 'warn' | 'fail' | 'unknown';
  message: string;
  path?: string;
  advice: string;
  steps: DiagnosticStep[];
}
export interface DiagnosticReport {
  id: string;
  generatedAt: number;
  environment: string;
  codexHome: string;
  dataPath: string;
  checks: DiagnosticCheck[];
}
export interface CodexLogFile {
  path: string;
  name: string;
  source: string;
  size: number;
  modifiedAt: number;
}
export interface CodexLogView {
  path: string;
  lines: string[];
  truncated: boolean;
  hints: DiagnosticCheck[];
}
export interface ToolboxLog {
  id: string;
  timestamp: number;
  level: 'info' | 'warn' | 'error';
  source: string;
  operationId?: string;
  message: string;
}
export interface LogSnapshot {
  entries: ToolboxLog[];
  directory?: string;
  writeError?: string;
  dropped: number;
  gap: boolean;
}
export async function diagnosticCall<T>(
  action: string,
  options: Record<string, unknown> = {},
): Promise<T> {
  if (!nativeAvailable) throw new Error('请在桌面版使用本机排查与日志功能。');
  try {
    return await invoke<T>('diagnostics', { request: { action, ...options } });
  } catch (error) {
    throw new Error(String(error));
  }
}
export function logsSnapshot(afterId?: string): Promise<LogSnapshot> {
  return invoke<LogSnapshot>('logs_snapshot', { afterId });
}
export function openLogs(operationId?: string) {
  window.dispatchEvent(new CustomEvent('toolbox-open-logs', { detail: { operationId } }));
}
