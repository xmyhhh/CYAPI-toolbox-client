import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import test from 'node:test';
import {
  isCreationSection,
  isToolPage,
  parseWorkbenchNavigation,
  WORKBENCH_KEY,
} from '../src/lib/workbenches.ts';

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');
const exists = (path: string) => existsSync(new URL(`../${path}`, import.meta.url));

test('workflows ship in standard builds with no research switch', () => {
  const pkg = JSON.parse(read('package.json'));
  assert.ok(pkg.scripts['desktop:exe']);
  assert.doesNotMatch(JSON.stringify(pkg.scripts), /research|workflow:service/);
  assert.doesNotMatch(read('src-tauri/Cargo.toml'), /research-workflows/);
  assert.doesNotMatch(read('src-tauri/src/lib.rs'), /research-workflows|configure_environment/);
  assert.equal(exists('src-tauri/tauri.research.conf.json'), false);
});

test('research directory contains only archived Markdown documents', () => {
  const files = readdirSync(new URL('../research', import.meta.url), {
    recursive: true,
    withFileTypes: true,
  });
  for (const file of files.filter((entry) => entry.isFile())) assert.match(file.name, /\.md$/);
  assert.ok(files.some((entry) => entry.name === 'README.md'));
});

test('the user workbench is a native React page; no remote UI and no child WebView', () => {
  const page = read('src/app/Workflows.tsx');
  assert.match(page, /import \{ Workbench \} from '\.\.\/features\/workflows\/Workbench'/);
  assert.match(page, /<Workbench\b/);
  for (const pattern of [/<iframe/, /postMessage/, /syncWorkflowView/, /invoke\(/, /WORKFLOW_URL/])
    assert.doesNotMatch(page, pattern);

  // The old remote UI and its native child WebView are gone.
  assert.equal(exists('src-tauri/src/workflows.rs'), false);
  assert.equal(exists('src-tauri/src/workflows-ui.js'), false);
  assert.equal(exists('src/lib/api/workflows.ts'), false);

  const backend = read('src-tauri/src/lib.rs');
  assert.doesNotMatch(backend, /mod workflows;|workflows::/);
  assert.match(backend, /fn trusted_caller\(view: &tauri::Webview\)/);
  assert.match(backend, /view\.label\(\) == "main"/);
  assert.match(backend, /local_app_url\(&url\)/);
  assert.match(backend, /invoke\.resolver\.reject/);

  const capability = JSON.parse(read('src-tauri/capabilities/default.json'));
  assert.deepEqual(capability.webviews, ['main']);
  assert.equal(capability.windows, undefined);
  assert.equal(capability.remote, undefined);

  // No `/workflows/` page path is left anywhere in the client.
  for (const file of ['src/app/Workflows.tsx', 'src/features/workflows/api.ts'])
    assert.doesNotMatch(read(file), /['"`]\/workflows\//);
});

test('the ComfyUI canvas is the only server-served page and it is sandboxed', () => {
  const canvas = read('src/features/workflows/CanvasDialog.tsx');
  assert.match(canvas, /<iframe/);
  assert.match(canvas, /src=\{url\}/);
  assert.match(canvas, /sandbox="allow-scripts allow-same-origin allow-downloads"/);
  assert.match(canvas, /canvasSession\(apiKey, kind, id\)/);
  assert.match(canvas, /WORKFLOW_ORIGIN/);

  const api = read('src/features/workflows/api.ts');
  const url = api.slice(api.indexOf('export function canvasUrl'), api.indexOf('export async function fetchMedia'));
  assert.match(url, /\/api\/workflows\/canvas\//);
  assert.match(url, /ct: token/);
  assert.match(url, /po: window\.location\.origin/);
  // The API Key never reaches the iframe URL.
  assert.doesNotMatch(url, /Bearer|apiKey|key:/);
});

test('workflow requests authenticate with a Bearer header and never use cookies or CSRF', () => {
  const api = read('src/features/workflows/api.ts');
  assert.match(api, /export const WORKFLOW_API = `\$\{SERVER\}\/api\/workflows`/);
  assert.match(api, /Authorization: `Bearer \$\{key\}`/);
  assert.doesNotMatch(api, /credentials:\s*['"]include/);
  assert.doesNotMatch(api, /csrf|X-CSRF|withCredentials/i);
  assert.doesNotMatch(api, /\/api\/workflows\/(ui|keyring)\b/);
});

test('workflow Keys live in the client and never in the service keyring', () => {
  const store = read('src/features/workflows/keyStore.ts');
  assert.match(store, /cyapi\.workflows\.keys\.v1/);
  assert.match(store, /localStorage\.setItem/);
  assert.match(store, /localStorage\.getItem/);
  assert.doesNotMatch(store, /\/api\//);
  assert.doesNotMatch(store, /document\.cookie/);

  const keys = read('src/features/workflows/KeysDialog.tsx');
  assert.match(keys, /保存到本机/);
  assert.match(keys, /localStorage/);
});

test('local Key store de-duplicates, filters temporary Keys and caps the list', async () => {
  const data = new Map<string, string>();
  (globalThis as { localStorage?: unknown }).localStorage = {
    getItem: (name: string) => data.get(name) ?? null,
    setItem: (name: string, value: string) => void data.set(name, value),
    removeItem: (name: string) => void data.delete(name),
  };
  const { addKey, loadKeys, saveKeys, maskKey } = await import(
    '../src/features/workflows/keyStore.ts'
  );

  assert.deepEqual(loadKeys(), { keys: [], activeId: null });

  const first = addKey([], ' 日常 ', ' sk-abcdef123456 ');
  assert.equal(first.name, '日常');
  assert.equal(first.key, 'sk-abcdef123456');
  assert.equal(maskKey(first.key), '•••• 3456');
  assert.equal(addKey([first], '重复', first.key), first, 'the same Key is not stored twice');

  saveKeys([first, { ...first, id: 'tmp', temporary: true }], first.id);
  const stored = loadKeys();
  assert.deepEqual(stored.keys.map((entry) => entry.id), [first.id]);
  assert.equal(stored.activeId, first.id);
  assert.deepEqual(loadKeys().keys[0].name, '日常');

  const many = Array.from({ length: 10 }, (_, index) => ({
    id: String(index),
    name: `Key ${index}`,
    key: `sk-${index}`,
    createdAt: index,
  }));
  assert.throws(() => addKey(many, 'extra', 'sk-extra'), /最多保存 10 个 Key/);
  assert.throws(() => addKey([], 'empty', '   '), /请输入 API Key/);

  delete (globalThis as { localStorage?: unknown }).localStorage;
});

test('desktop CSP allows the canvas origin and blob media, and nothing broader', () => {
  const csp = JSON.parse(read('src-tauri/tauri.conf.json')).app.security.csp as string;
  const directive = (name: string) => csp.split(';').map((part) => part.trim()).find((part) => part.startsWith(`${name} `)) || '';
  const frame = directive('frame-src');
  const connect = directive('connect-src');
  const img = directive('img-src');
  const media = directive('media-src');
  for (const origin of ['https://api.cyapi.cyou', 'http://127.0.0.1:18733']) {
    assert.ok(frame.includes(origin), `frame-src must allow ${origin}`);
    assert.ok(connect.includes(origin), `connect-src must allow ${origin}`);
  }
  assert.ok(img.includes('blob:'));
  assert.ok(media.includes('blob:'));
  assert.doesNotMatch(frame, /frame-src 'none'/);
  assert.doesNotMatch(frame, /\s\*\s*;|frame-src \*/);
  assert.doesNotMatch(csp, /unsafe-eval/);
});

test('media is fetched with the Bearer Key and shown through blob URLs', () => {
  const media = read('src/features/workflows/Media.tsx');
  assert.match(media, /fetchMedia\(/);
  assert.match(media, /URL\.createObjectURL/);
  assert.match(media, /URL\.revokeObjectURL/);
  assert.doesNotMatch(media, /[?&]mt=/);

  const api = read('src/features/workflows/api.ts');
  assert.match(api, /export async function fetchMedia/);
  assert.match(api, /export async function downloadArtifact/);
  assert.doesNotMatch(api, /\/download-token|mediaToken|\bmt=/);
});

test('business workbenches remember independent pages with no research storage namespace', () => {
  const defaults = { workbench: 'create', toolPage: 'codex', creationSection: 'library' };
  for (const saved of [null, 'invalid', 'null', '[]', '123'])
    assert.deepEqual(parseWorkbenchNavigation(saved), defaults);
  assert.deepEqual(
    parseWorkbenchNavigation(
      JSON.stringify({ workbench: 'tools', toolPage: 'mcp', creationSection: 'results' }),
    ),
    { workbench: 'tools', toolPage: 'mcp', creationSection: 'results' },
  );
  assert.deepEqual(
    parseWorkbenchNavigation(
      JSON.stringify({ workbench: 'claude', toolPage: 'settings', creationSection: '<script>' }),
    ),
    defaults,
  );
  assert.equal(isCreationSection('editor'), false);
  assert.equal(isToolPage('workflows'), false);
  assert.equal(WORKBENCH_KEY, 'cyapi.workbenches.v1');
});

test('Tools and Workflows are peers and the workbench keeps the App sidebar contract', () => {
  const app = read('src/app/App.tsx');
  const tools = app.indexOf("['tools', SquareTerminal, '工具接入'");
  const workflows = app.indexOf("['create', Workflow, '工作流'");
  assert.ok(tools >= 0 && workflows > tools);
  assert.match(app, /hidden=\{page !== 'workflows'\}/);
  assert.match(app, /visible=\{page === 'workflows' && !modalOpen && !logsOpen\}/);
  assert.match(app, /workbench !== 'tools' \|\| toolsLoaded\.current/);
  assert.doesNotMatch(app, /researchBuild|预研|RESEARCH/);

  const workbench = read('src/features/workflows/Workbench.tsx');
  for (const prop of ['onSectionChange', 'onLockedChange', 'initialSection', 'navigation'])
    assert.match(workbench, new RegExp(`${prop}`));
  assert.match(workbench, /changeSection\('queue'\)/);
  assert.doesNotMatch(workbench, /<iframe/);
});

test('desktop workflow copy never claims server mode or shares tool credentials', () => {
  const app = read('src/app/App.tsx');
  assert.match(app, /费用以运行前报价为准/);
  assert.match(app, /工作流使用独立的 CYAPI API Key/);
  assert.match(app, /任务状态待核对时/);
  assert.doesNotMatch(app, /参考图拼贴模拟 · 非 AI 生成 · 不计费/);

  const guide = read('src/features/workflows/Dialogs.tsx');
  assert.match(guide, /Key 只保存在本机客户端/);
  assert.match(guide, /画布是唯一从服务端加载的界面/);
});
