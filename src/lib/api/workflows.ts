import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable } from './native';
import type { CreationSection } from '../workbenches';

export const WORKFLOW_URL = 'https://api.cyapi.cyou/workflows/';
export const reloadWorkflowView = () => invoke<void>('reload_workflows');
export interface WorkflowLayout {
  visible: boolean;
  x: number;
  y: number;
  width: number;
  height: number;
  section: CreationSection;
  theme: 'dark' | 'light';
  navigationId: string;
}
export interface WorkflowView {
  url: string;
  view: { ready: boolean; section: CreationSection | null; locked: boolean };
}

// Preserve hide/show ordering across React effects and native async creation.
let pending: Promise<unknown> = Promise.resolve();
export function syncWorkflowView(layout: WorkflowLayout): Promise<WorkflowView> {
  if (!nativeAvailable) return Promise.reject(new Error('请使用桌面客户端或直接打开工作流服务。'));
  const next = pending.catch(() => {}).then(() => invoke<WorkflowView>('workflows', { layout }));
  pending = next;
  return next;
}
