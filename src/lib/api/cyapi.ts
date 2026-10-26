import { invoke } from '@tauri-apps/api/core';
import { nativeAvailable, SERVER_URL } from './native';
import { createCyapiRoutesStore, defaultCyapiRoutes, validCyapiManifest } from '../cyapi-routes';
import type { CyapiRoutesView } from '../cyapi-routes';
export { CYAPI_DEFAULT_URL, sameRoute } from '../cyapi-routes';
export type { CyapiRoute, CyapiManifest, CyapiRoutesView } from '../cyapi-routes';
export interface CyapiMeasurement {
  routeId: string;
  latencyMs: number | null;
  reachable: boolean;
  message: string;
}
const CACHE_KEY = 'cyapi-toolbox.routes.v1';
async function readLocalRoutes(): Promise<CyapiRoutesView> {
  if (nativeAvailable) return invoke('cyapi_routes', { action: 'cache', routeId: null });
  try {
    const manifest: unknown = JSON.parse(localStorage.getItem(CACHE_KEY) ?? 'null');
    if (validCyapiManifest(manifest)) return { manifest, source: 'cache' };
  } catch {
    // Browser storage is optional; desktop routes are persisted by Rust.
  }
  return defaultCyapiRoutes;
}
async function fetchCyapiRoutes(): Promise<CyapiRoutesView> {
  if (nativeAvailable) return invoke('cyapi_routes', { action: 'read', routeId: null });
  const response = await fetch(`${SERVER_URL}/api/cyapi-routes`, {
    cache: 'no-store',
    signal: AbortSignal.timeout(8000),
  });
  if (!response.ok) throw new Error('unavailable');
  const manifest: unknown = await response.json();
  if (!validCyapiManifest(manifest)) throw new Error('invalid');
  try {
    // Only the public manifest is stored; never API keys, balances or probes.
    localStorage.setItem(CACHE_KEY, JSON.stringify(manifest));
  } catch {
    // An unavailable browser cache does not discard a valid server response.
  }
  return { manifest, source: 'server' };
}

const routesStore = createCyapiRoutesStore({
  readCache: readLocalRoutes,
  refresh: fetchCyapiRoutes,
});
export const cyapiRoutesSnapshot = routesStore.getSnapshot;
export const subscribeCyapiRoutes = routesStore.subscribe;
export const readCyapiRoutes = routesStore.read;
export async function probeCyapiRoute(routeId: string): Promise<CyapiMeasurement> {
  if (!nativeAvailable) throw new Error('请在桌面版测速');
  return invoke('cyapi_routes', { action: 'probe', routeId });
}
