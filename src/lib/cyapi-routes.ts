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
  message?: string | null;
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
  const normalize = (value: string) => {
    const trimmed = value.trim().replace(/\/+$/, '');
    try {
      return new URL(trimmed).href.replace(/\/+$/, '');
    } catch {
      return trimmed;
    }
  };
  return normalize(left) === normalize(right);
}

export function validCyapiManifest(value: unknown): value is CyapiManifest {
  if (!value || typeof value !== 'object') return false;
  const manifest = value as CyapiManifest;
  if (
    manifest.schema !== 1 ||
    manifest.defaultRouteId !== 'official' ||
    !Array.isArray(manifest.routes) ||
    !manifest.routes.length ||
    manifest.routes.length > 64
  )
    return false;
  const ids = new Set<string>();
  const urls = new Set<string>();
  for (const route of manifest.routes) {
    if (
      !route ||
      typeof route.id !== 'string' ||
      !/^[a-z0-9-]{1,64}$/.test(route.id) ||
      ids.has(route.id) ||
      typeof route.name !== 'string' ||
      !route.name.trim() ||
      Array.from(route.name).length > 80 ||
      /[\x00-\x1f\x7f]/.test(route.name) ||
      typeof route.baseUrl !== 'string' ||
      route.baseUrl.length > 2048 ||
      /[\s\x00-\x1f\x7f]/.test(route.baseUrl)
    )
      return false;
    try {
      const url = new URL(route.baseUrl);
      const normalized = url.href.replace(/\/+$/, '');
      if (
        !['http:', 'https:'].includes(url.protocol) ||
        !url.hostname ||
        url.username ||
        url.password ||
        url.search ||
        url.hash ||
        urls.has(normalized)
      )
        return false;
      urls.add(normalized);
    } catch {
      return false;
    }
    ids.add(route.id);
  }
  return manifest.routes.some((r) => r.id === 'official' && r.baseUrl === CYAPI_DEFAULT_URL);
}

interface RoutesSnapshot {
  view: CyapiRoutesView;
  loading: boolean;
}

// One shared snapshot for all workspaces. Hydration is local-only; an automatic
// refresh follows once per launch, and manual refreshes reuse in-flight work.
export function createCyapiRoutesStore(dependencies: {
  readCache: () => Promise<CyapiRoutesView>;
  refresh: () => Promise<CyapiRoutesView>;
}) {
  let snapshot: RoutesSnapshot = { view: defaultCyapiRoutes, loading: true };
  let hydrated = false;
  let started = false;
  let request: Promise<CyapiRoutesView> | undefined;
  const listeners = new Set<() => void>();
  function publish(view: CyapiRoutesView, loading: boolean) {
    snapshot = { view, loading };
    listeners.forEach((listener) => listener());
  }
  function fallback(): CyapiRoutesView {
    return snapshot.view.source === 'default'
      ? {
          ...defaultCyapiRoutes,
          message: '暂时无法获取备用线路，可使用官方默认线路或稍后更新。',
        }
      : {
          ...snapshot.view,
          source: 'cache',
          message: '暂时无法更新，使用上次保存的线路清单。',
        };
  }
  return {
    getSnapshot: () => snapshot,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    read(options?: { force?: boolean }): Promise<CyapiRoutesView> {
      if (request) return request;
      if (started && !options?.force) return Promise.resolve(snapshot.view);
      started = true;
      publish(snapshot.view, true);
      const pending = (async () => {
        if (!hydrated) {
          try {
            const cached = await dependencies.readCache();
            if (cached.source === 'cache' && validCyapiManifest(cached.manifest))
              publish(cached, true);
          } catch {
            // Missing/corrupt local data must not prevent a fresh server read.
          }
          hydrated = true;
        }
        let next: CyapiRoutesView;
        try {
          next = await dependencies.refresh();
          if (!validCyapiManifest(next.manifest)) throw new Error('Invalid routes');
          if (next.source === 'default' && snapshot.view.source !== 'default') next = fallback();
        } catch {
          next = fallback();
        }
        publish(next, false);
        return next;
      })().finally(() => {
        if (request === pending) request = undefined;
      });
      request = pending;
      return pending;
    },
  };
}
