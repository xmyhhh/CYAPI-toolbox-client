import assert from 'node:assert/strict';
import { readFileSync, readdirSync, existsSync } from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
import {
  isCreationSection,
  isToolPage,
  parseWorkbenchNavigation,
  WORKBENCH_KEY,
} from '../src/lib/workbenches.ts';

const read = (path: string) => readFileSync(new URL(`../${path}`, import.meta.url), 'utf8');

test('workflows are in standard builds with the existing Service and no research switch', () => {
  const pkg = JSON.parse(read('package.json'));
  assert.ok(pkg.scripts['desktop:exe']);
  assert.doesNotMatch(JSON.stringify(pkg.scripts), /research|workflow:service/);
  assert.doesNotMatch(read('src-tauri/Cargo.toml'), /research-workflows/);
  assert.doesNotMatch(read('src-tauri/src/lib.rs'), /research-workflows|configure_environment/);
  assert.match(read('src-tauri/src/lib.rs'), /mod workflows;/);
  assert.match(read('src-tauri/src/service.rs'), /fn workflow_url\(\)[\s\S]*?SERVER_URL/);
  assert.equal(
    existsSync(new URL('../src-tauri/tauri.research.conf.json', import.meta.url)),
    false,
  );
});

test('research directory contains only archived Markdown documents', () => {
  const files = readdirSync(new URL('../research', import.meta.url), {
    recursive: true,
    withFileTypes: true,
  });
  for (const file of files.filter((entry) => entry.isFile())) assert.match(file.name, /\.md$/);
  assert.ok(files.some((entry) => entry.name === 'README.md'));
});

test('child view has no local capabilities and every app command rejects remote callers', () => {
  const capability = JSON.parse(read('src-tauri/capabilities/default.json'));
  assert.deepEqual(capability.webviews, ['main']);
  assert.equal(capability.windows, undefined);
  assert.equal(capability.remote, undefined);
  const backend = read('src-tauri/src/lib.rs');
  assert.match(backend, /trusted_caller\(invoke\.message\.webview_ref\(\)\)/);
  assert.match(backend, /invoke\.resolver\.reject/);
  const workflows = read('src-tauri/src/workflows.rs');
  assert.match(workflows, /view\.label\(\) == "main"/);
  assert.match(workflows, /view\.url\(\)\.is_ok_and/);
  assert.match(workflows, /NewWindowResponse::Deny/);
  assert.match(workflows, /PermissionResponse::Deny/);
  assert.doesNotMatch(
    workflows,
    /Command::new|ChildStdin|\.shutdown\(|CYAPI_WORKFLOW_SERVICE_URL|apiKey|authToken/,
  );
});

test('remote service is first-party WebView, not iframe or a local engine probe', () => {
  const config = JSON.parse(read('src-tauri/tauri.conf.json'));
  assert.match(config.app.security.csp, /frame-src 'none'/);
  assert.doesNotMatch(config.app.security.csp, /api\.cyapi|127\.0\.0\.1:8188|\*/);
  const backend = read('src-tauri/src/workflows.rs');
  assert.match(backend, /window\s*\.add_child/);
  assert.match(backend, /WebviewUrl::External\(crate::service::Service::workflow_url\(\)\)/);
  assert.match(backend, /\.on_navigation\(allowed_navigation\)/);
  assert.match(backend, /\.on_download/);
  assert.match(backend, /allowed_download\(&url\)/);
  assert.doesNotMatch(
    read('src/app/Workflows.tsx'),
    /<iframe|postMessage|serviceOrigin|readWorkflows|enginePid/,
  );
  assert.match(read('src/lib/api/workflows.ts'), /https:\/\/api\.cyapi\.cyou\/workflows\//);
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

test('Tools and Workflows are peers and remote view hides rather than reloads', () => {
  const app = read('src/app/App.tsx');
  const tools = app.indexOf("['tools', SquareTerminal, '工具接入'");
  const workflows = app.indexOf("['create', Workflow, '工作流'");
  assert.ok(tools >= 0 && workflows > tools);
  assert.match(app, /hidden=\{page !== 'workflows'\}/);
  assert.match(app, /visible=\{page === 'workflows' && !modalOpen && !logsOpen\}/);
  assert.match(app, /workbench !== 'tools' \|\| toolsLoaded\.current/);
  assert.doesNotMatch(app, /researchBuild|预研|RESEARCH|本机模拟/);
});

// Execute the actual native-injected adapter against a tiny fake DOM. No remote
// site, production key, paid call, engine, browser or credential fixture needed.
function adapter({ frame = false, origin = 'https://api.cyapi.cyou' } = {}) {
  let active: string | null = 'library',
    modal = false,
    login = false,
    busy = false;
  const clicks: string[] = [];
  const dataset: Record<string, string> = {};
  const window: Record<string, unknown> = {};
  window.top = frame ? {} : window;
  const context = vm.createContext({
    window,
    location: { origin, pathname: '/workflows/' },
    document: {
      documentElement: { dataset },
      querySelector(selector: string) {
        if (selector.startsWith('#modal')) return modal || busy ? {} : null;
        if (selector === '#login-form') return login ? {} : null;
        if (selector === '.tabs .active') return active ? { dataset: { action: active } } : null;
        const section = /data-action="(\w+)"/.exec(selector)?.[1];
        if (section)
          return {
            click() {
              clicks.push(section);
              active = section;
            },
          };
        return null;
      },
    },
  });
  vm.runInContext(read('src-tauri/src/workflows-ui.js'), context);
  const sync = (section = 'library', navigationId = 'nav-1', theme = 'light') =>
    JSON.parse(
      JSON.stringify(
        (window.__cyapiWorkflowUi as { sync: (value: unknown) => unknown })?.sync({
          section,
          navigationId,
          theme,
        }),
      ),
    );
  return {
    window,
    sync,
    clicks,
    dataset,
    setActive: (next: string | null) => {
      active = next;
    },
    setModal: (next: boolean) => {
      modal = next;
    },
    setLogin: (next: boolean) => {
      login = next;
    },
    setBusy: (next: boolean) => {
      busy = next;
    },
  };
}

test('UI adapter accepts only fixed section and theme; never exposes native bridge or reads secrets', () => {
  const code = read('src-tauri/src/workflows-ui.js');
  assert.doesNotMatch(
    code,
    /__TAURI|invoke\(|fetch\(|\.cookie|localStorage|api-key|\.value|message\.(action|command|url)/,
  );
  for (const options of [{ frame: true }, { origin: 'https://example.com' }])
    assert.equal(adapter(options).window.__cyapiWorkflowUi, undefined);
  const ui = adapter();
  assert.equal(ui.sync('editor').ready, false);
  assert.equal(ui.sync('library', 'nav-1', 'invalid').ready, false);
  assert.deepEqual(ui.clicks, []);
});

test('UI adapter navigates via service action, honours locks, never replays requests', () => {
  const ui = adapter();
  assert.deepEqual(ui.sync(), { ready: true, section: 'library', locked: false });
  ui.sync('drafts', 'nav-2', 'dark');
  assert.deepEqual(ui.clicks, ['drafts']);
  assert.equal(ui.dataset.theme, 'dark');
  ui.setActive('queue'); // navigation inside remote app must win over old request
  ui.sync('drafts', 'nav-2');
  assert.deepEqual(ui.clicks, ['drafts']);
  ui.setModal(true);
  assert.equal(ui.sync('results', 'nav-3').locked, true);
  assert.deepEqual(ui.clicks, ['drafts']);
  ui.setModal(false);
  ui.setBusy(true);
  ui.sync('results', 'nav-3');
  assert.deepEqual(ui.clicks, ['drafts']);
  ui.setBusy(false);
  assert.equal(ui.sync('results', 'nav-3').section, 'results');
  assert.deepEqual(ui.clicks, ['drafts', 'results']);
});

test('UI adapter defers navigation until after manual login without touching Key field', () => {
  const ui = adapter();
  ui.setActive(null);
  ui.setLogin(true);
  assert.deepEqual(ui.sync('results'), { ready: true, section: null, locked: false });
  assert.deepEqual(ui.clicks, []);
  ui.setLogin(false);
  ui.setActive('library');
  assert.equal(ui.sync('results').section, 'results');
  assert.deepEqual(ui.clicks, ['results']);
});
