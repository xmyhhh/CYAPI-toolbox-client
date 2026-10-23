import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable } from './native';

export interface CyapiBalancePlan {
  planName: string;
  isValid: boolean;
  total?: number;
  used?: number;
  remaining?: number;
  unit: string;
  extra?: string;
}

export async function readCyapiBalance(baseUrl: string, apiKey: string) {
  if (!nativeAvailable) throw new Error('请在桌面版查询余额');
  return invoke<CyapiBalancePlan[]>('cyapi_balance', { baseUrl, apiKey });
}
