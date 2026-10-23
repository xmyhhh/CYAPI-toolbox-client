import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable, SERVER_URL } from './native';

export const CYAPI_DEFAULT_URL = 'https://www.cyapi.cyou/v1';
export interface CyapiRoute {
  id: string;
  name: string;
  baseUrl: string;
}
export interface CyapiManifest {
  schema: 1;
  defaultRouteId: string;
  routes: CyapiRoute[];
}
export interface CyapiRoutesView {
  manifest: CyapiManifest;
  source: 'server' | 'cache' | 'default';
  message?: string;
}
export interface CyapiMeasurement {
  routeId: string;
  latencyMs: number | null;
  reachable: boolean;
  message: string;
}
export const defaultCyapiRoutes: CyapiRoutesView = {
  source: 'default',
  manifest: {
    schema: 1,
    defaultRouteId: 'official',
    routes: [{ id: 'official', name: 'CYAPI 官方线路', baseUrl: CYAPI_DEFAULT_URL }],
  },
};
export function sameRoute(left: string, right: string) {
  return left.trim().replace(/\/+$/, '') === right.trim().replace(/\/+$/, '');
}
// The route manifest is fetched once per application session. Page/workspace
// switches reuse this snapshot; only an explicit "update routes" click forces
// another server request.
let routesCache: CyapiRoutesView | undefined;
let routesRequest: Promise<CyapiRoutesView> | undefined;

async function fetchCyapiRoutes(): Promise<CyapiRoutesView> {
  if (nativeAvailable) return invoke('cyapi_routes', { action: 'read', routeId: null });
  try {
    const response = await fetch(`${SERVER_URL}/api/cyapi-routes`, {
      signal: AbortSignal.timeout(8000),
    });
    if (!response.ok) throw new Error('unavailable');
    const manifest = (await response.json()) as CyapiManifest;
    if (
      manifest.schema !== 1 ||
      manifest.defaultRouteId !== 'official' ||
      !Array.isArray(manifest.routes) ||
      manifest.routes.length > 64 ||
      !manifest.routes.every(
        (r) =>
          typeof r.id === 'string' && typeof r.name === 'string' && typeof r.baseUrl === 'string',
      ) ||
      !manifest.routes.some((r) => r.id === 'official' && r.baseUrl === CYAPI_DEFAULT_URL)
    )
      throw new Error('invalid');
    return { manifest, source: 'server' };
  } catch {
    return {
      ...defaultCyapiRoutes,
      message: '暂时无法获取备用线路，可使用官方默认线路或稍后更新。',
    };
  }
}

export function cachedCyapiRoutes(): CyapiRoutesView | undefined {
  return routesCache;
}

export async function readCyapiRoutes(options?: { force?: boolean }): Promise<CyapiRoutesView> {
  if (routesCache && !options?.force) return routesCache;
  if (routesRequest) return routesRequest;
  const pending = fetchCyapiRoutes()
    .then((view) => {
      routesCache = view;
      return view;
    })
    .finally(() => {
      if (routesRequest === pending) routesRequest = undefined;
    });
  routesRequest = pending;
  return pending;
}
export async function probeCyapiRoute(routeId: string): Promise<CyapiMeasurement> {
  if (!nativeAvailable) throw new Error('请在桌面版测速');
  return invoke('cyapi_routes', { action: 'probe', routeId });
}
