import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable } from './native';
import type { UsageReport, UsageWorkspace } from '../usage';

export async function readUsage(workspace: UsageWorkspace, refresh = false): Promise<UsageReport> {
  if (!nativeAvailable) throw new Error('请在桌面版读取本机用量记录。');
  try {
    return await invoke<UsageReport>('usage_stats', { workspace, refresh });
  } catch (error) {
    throw new Error(String(error));
  }
}
