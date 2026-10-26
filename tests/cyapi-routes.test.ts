import test from 'node:test';
import assert from 'node:assert/strict';
import {
  createCyapiRoutesStore,
  defaultCyapiRoutes,
  sameRoute,
  validCyapiManifest,
} from '../src/lib/cyapi-routes.ts';
import type { CyapiManifest, CyapiRoutesView } from '../src/lib/cyapi-routes.ts';

function manifest(id = 'backup'): CyapiManifest {
  return {
    ...defaultCyapiRoutes.manifest,
    routes: [
      ...defaultCyapiRoutes.manifest.routes,
      { id, name: '备用线路', baseUrl: `https://${id}.example.test:4080/v1` },
    ],
  };
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}

test('hydrates local routes and matches saved configuration before the automatic refresh completes', async () => {
  const refresh = deferred<CyapiRoutesView>();
  const local = manifest();
  let reads = 0;
  let refreshes = 0;
  const store = createCyapiRoutesStore({
    readCache: async () => {
      reads++;
      return { manifest: local, source: 'cache' };
    },
    refresh: () => {
      refreshes++;
      return refresh.promise;
    },
  });
  let updates = 0;
  const unsubscribe = store.subscribe(() => {
    updates++;
  });
  const pending = store.read();
  await Promise.resolve();
  assert.equal(store.getSnapshot().loading, true);
  assert.equal(store.getSnapshot().view.source, 'cache');
  const configured = ' https://BACKUP.example.test:4080/v1/ ';
  assert.equal(
    store.getSnapshot().view.manifest.routes.find((r) => sameRoute(r.baseUrl, configured))?.id,
    'backup',
  );
  assert.strictEqual(store.read(), pending);
  assert.strictEqual(store.read({ force: true }), pending);
  assert.equal(reads, 1);
  assert.equal(refreshes, 1);
  refresh.resolve({ manifest: manifest('new-backup'), source: 'server' });
  await pending;
  assert.equal(store.getSnapshot().loading, false);
  assert.deepEqual(
    store.getSnapshot().view.manifest.routes.map((r) => r.id),
    ['official', 'new-backup'],
  );
  assert.equal(configured, ' https://BACKUP.example.test:4080/v1/ ');
  assert.ok(updates >= 3);
  unsubscribe();
  await store.read();
  assert.equal(refreshes, 1, 'page/workspace switches must reuse the refreshed snapshot');
});

test('failed automatic/manual refreshes retain the last usable routes and manual success replaces them', async () => {
  let online = false;
  let calls = 0;
  const store = createCyapiRoutesStore({
    readCache: async () => ({ manifest: manifest(), source: 'cache' }),
    refresh: async () => {
      calls++;
      if (!online) throw new Error('offline');
      return { manifest: manifest('replacement'), source: 'server' };
    },
  });
  await store.read();
  assert.equal(store.getSnapshot().view.manifest.routes[1].id, 'backup');
  assert.match(store.getSnapshot().view.message!, /上次保存/);
  await store.read({ force: true });
  assert.equal(store.getSnapshot().view.manifest.routes[1].id, 'backup');
  online = true;
  const pending = store.read({ force: true });
  assert.equal(store.getSnapshot().view.manifest.routes[1].id, 'backup');
  await pending;
  assert.equal(store.getSnapshot().view.manifest.routes[1].id, 'replacement');
  assert.equal(store.getSnapshot().view.message, undefined);
  assert.equal(calls, 3);
});

test('does not discard hydrated routes when the native refresh returns only the built-in default', async () => {
  const store = createCyapiRoutesStore({
    readCache: async () => ({ manifest: manifest(), source: 'cache' }),
    refresh: async () => defaultCyapiRoutes,
  });
  const view = await store.read();
  assert.equal(view.source, 'cache');
  assert.equal(view.manifest.routes[1].id, 'backup');
});

test('fresh launches hydrate the newly persisted manifest then automatically refresh again', async () => {
  let disk = manifest();
  let requests = 0;
  const launch = () =>
    createCyapiRoutesStore({
      readCache: async () => ({ manifest: disk, source: 'cache' }),
      refresh: async () => {
        requests++;
        if (requests === 1) {
          disk = manifest('updated');
          return { manifest: disk, source: 'server' };
        }
        throw new Error('offline after restart');
      },
    });
  await launch().read();
  const restarted = launch();
  await restarted.read();
  assert.equal(requests, 2);
  assert.equal(restarted.getSnapshot().view.manifest.routes[1].id, 'updated');
  assert.equal(restarted.getSnapshot().view.source, 'cache');
});

test('missing/corrupt caches do not prevent recovery and first offline launch keeps the official route', async () => {
  const store = createCyapiRoutesStore({
    readCache: async () => {
      throw new Error('corrupt');
    },
    refresh: async () => {
      throw new Error('offline');
    },
  });
  const view = await store.read();
  assert.equal(view.source, 'default');
  assert.deepEqual(view.manifest, defaultCyapiRoutes.manifest);
  assert.equal(store.getSnapshot().loading, false);
  const recovered = createCyapiRoutesStore({
    readCache: async () => ({ manifest: null as unknown as CyapiManifest, source: 'cache' }),
    refresh: async () => ({ manifest: manifest(), source: 'server' }),
  });
  assert.equal((await recovered.read()).source, 'server');
});

test('invalid fresh responses leave validated cache intact', async () => {
  const store = createCyapiRoutesStore({
    readCache: async () => ({ manifest: manifest(), source: 'cache' }),
    refresh: async () => ({ manifest: {} as CyapiManifest, source: 'server' }),
  });
  assert.equal((await store.read()).manifest.routes[1].id, 'backup');
});

test('validates cached/downloaded public routes without accepting credentials or duplicate addresses', () => {
  assert.equal(validCyapiManifest(manifest()), true);
  for (const value of [null, {}, { ...manifest(), schema: 2 }, { ...manifest(), routes: [] }])
    assert.equal(validCyapiManifest(value), false);
  for (const badUrl of [
    'https://user:secret@example.test/v1',
    'https://example.test/v1?token=secret',
    'https://example.test/v1#fragment',
    'file:///secret',
    `${defaultCyapiRoutes.manifest.routes[0].baseUrl}/`,
  ]) {
    const value = manifest();
    value.routes[1].baseUrl = badUrl;
    assert.equal(validCyapiManifest(value), false, badUrl);
  }
  const duplicate = manifest();
  duplicate.routes[1].id = 'official';
  assert.equal(validCyapiManifest(duplicate), false);
});

test('route matching normalizes host casing, default ports and trailing slashes without changing paths or schemes', () => {
  assert.equal(sameRoute('https://EXAMPLE.test:443/v1/', ' https://example.test/v1 '), true);
  assert.equal(sameRoute('http://example.test/v1', 'https://example.test/v1'), false);
  assert.equal(sameRoute('https://example.test/V1', 'https://example.test/v1'), false);
});
