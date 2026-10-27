// Real Windows EXE + WebView2 smoke check. Uses a fresh temporary user profile,
// no Agent credentials, no valid Key, no paid model and no service mutations.
// CDP is enabled only on this child process and removed when the EXE exits.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, writeFile } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { setTimeout as delay } from 'node:timers/promises';

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const executable = path.resolve(
  root,
  process.argv[2] || 'src-tauri/target/release/cyapi-toolbox.exe',
);
if (process.platform !== 'win32') throw new Error('This smoke check requires Windows/WebView2.');
const output = path.join(root, 'output', 'workflows-desktop-check', crypto.randomUUID());
await mkdir(output, { recursive: true });
const port = 19431;
try {
  await fetch(`http://127.0.0.1:${port}/json/list`);
  throw new Error(`CDP port ${port} is already in use; refusing to attach to another browser.`);
} catch (error) {
  if (!error.cause) throw error;
}
const child = spawn(executable, [], {
  cwd: root,
  env: {
    ...process.env,
    CYAPI_TOOLBOX_HOME_DIR: path.join(output, 'home'),
    CYAPI_TOOLBOX_DATA_DIR: path.join(output, 'data'),
    WEBVIEW2_USER_DATA_FOLDER: path.join(output, 'webview'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  },
  stdio: 'ignore',
});
const connections = [];
const checks = [];
const check = (name, value) => {
  assert.ok(value, name);
  checks.push(name);
  console.log(`PASS ${name}`);
};
async function until(fn, timeout = 30000) {
  const end = Date.now() + timeout;
  while (Date.now() < end) {
    const value = await fn();
    if (value) return value;
    await delay(200);
  }
  throw new Error('Timed out waiting for desktop condition');
}
async function pages() {
  try {
    return await (await fetch(`http://127.0.0.1:${port}/json/list`)).json();
  } catch {
    return [];
  }
}
async function connect(target) {
  const ws = new WebSocket(target.webSocketDebuggerUrl);
  await new Promise((resolve, reject) => {
    ws.addEventListener('open', resolve, { once: true });
    ws.addEventListener('error', reject, { once: true });
  });
  let id = 0;
  const pending = new Map();
  ws.addEventListener('message', ({ data }) => {
    const result = JSON.parse(data);
    const task = pending.get(result.id);
    if (!task) return;
    pending.delete(result.id);
    clearTimeout(task.timer);
    if (result.error) task.reject(new Error(result.error.message));
    else task.resolve(result.result);
  });
  const call = (method, params = {}) =>
    new Promise((resolve, reject) => {
      const next = ++id;
      const timer = setTimeout(() => {
        pending.delete(next);
        reject(new Error(`CDP ${method} timed out`));
      }, 15000);
      pending.set(next, { resolve, reject, timer });
      ws.send(JSON.stringify({ id: next, method, params }));
    });
  connections.push(ws);
  const evaluate = async (expression) => {
    const result = await call('Runtime.evaluate', {
      expression,
      returnByValue: true,
      awaitPromise: true,
    });
    if (result.exceptionDetails)
      throw new Error(
        result.exceptionDetails.text + ': ' + result.exceptionDetails.exception?.description,
      );
    return result.result.value;
  };
  return { call, evaluate };
}

try {
  const local = await until(async () =>
    (await pages()).find((page) => page.url.startsWith('http://tauri.localhost')),
  );
  const main = await connect(local);
  await until(() => main.evaluate(`Boolean(document.querySelector('.n-workbenches'))`));
  check(
    'standard client exposes peer Tools / Workflows entries',
    await main.evaluate(
      `Array.from(document.querySelectorAll('.n-workbenches strong')).map(el=>el.textContent).join('|') === '工具接入|工作流'`,
    ),
  );
  // Saved navigation is absent in this isolated profile; default workflow must
  // load without starting a local engine or selecting/configuring an Agent.
  const target = await until(
    async () => (await pages()).find((page) => page.url === 'https://api.cyapi.cyou/workflows/'),
    60000,
  );
  const remote = await connect(target);
  await until(() => remote.evaluate(`Boolean(document.querySelector('#login-form'))`), 60000);
  check(
    'top-level remote HTTPS login loads inside main window',
    await remote.evaluate(
      `window.top===window && location.origin==='https://api.cyapi.cyou' && !document.querySelector('#app iframe')`,
    ),
  );
  await until(() => remote.evaluate(`document.documentElement.dataset.embedded === 'true'`));
  check(
    'native UI adapter applies appearance without parent iframe',
    await remote.evaluate(`['light','dark'].includes(document.documentElement.dataset.theme)`),
  );
  const denied = await remote.evaluate(
    `(async()=>{try{await window.__TAURI_INTERNALS__.invoke('toolbox',{request:{action:'status'}});return false}catch{return true}})()`,
  );
  check('remote content cannot invoke toolbox commands', denied);
  check(
    'remote content cannot invoke core window commands',
    await remote.evaluate(
      `(async()=>{try{await window.__TAURI_INTERNALS__.invoke('plugin:window|get_all_windows');return false}catch{return true}})()`,
    ),
  );
  check(
    'remote content cannot invoke workflow layout command',
    await remote.evaluate(
      `(async()=>{try{await window.__TAURI_INTERNALS__.invoke('workflows',{layout:{visible:false,x:0,y:0,width:0,height:0,section:'library',theme:'light',navigationId:'test'}});return false}catch{return true}})()`,
    ),
  );
  check(
    'local core permissions still work with a child view',
    typeof (await main.evaluate(`window.__TAURI_INTERNALS__.invoke('plugin:app|version')`)) ===
      'string',
  );
  check(
    'unauthenticated private endpoint remains 401',
    (await remote.evaluate(`fetch('/api/workflows/state').then(res=>res.status)`)) === 401,
  );
  await remote.evaluate(
    `document.querySelector('#api-key').value='invalid-desktop-smoke-key';document.querySelector('#login-form').requestSubmit();true`,
  );
  await until(
    () => remote.evaluate(`Boolean(document.querySelector('#login-error')?.textContent)`),
    45000,
  );
  check(
    'invalid Key is rejected and cleared from input',
    await remote.evaluate(
      `document.querySelector('#api-key').value==='' && Boolean(document.querySelector('#login-error').textContent)`,
    ),
  );
  // Only this test-owned, non-secret UI token is set to detect unwanted reload.
  await remote.evaluate(`window.__desktopTestMarker='retained';true`);
  await main.evaluate(`document.querySelector('[aria-label="设置"]').click();true`);
  await until(() => main.evaluate(`Boolean(document.querySelector('.n-settings'))`));
  check(
    'settings opens without destroying remote page',
    await remote.evaluate(`window.__desktopTestMarker==='retained'`),
  );
  await main.evaluate(`document.querySelector('.n-settings-theme:nth-child(3)').click();true`);
  await main.evaluate(
    `Array.from(document.querySelectorAll('.n-workbenches button')).find(el=>el.textContent.includes('工作流')).click();true`,
  );
  await until(() => remote.evaluate(`document.documentElement.dataset.theme==='dark'`));
  check(
    'workflow restores same WebView with global dark appearance',
    await remote.evaluate(`window.__desktopTestMarker==='retained'`),
  );
  const layout = await main.evaluate(
    `(()=>{const r=document.querySelector('.wf-workspace').getBoundingClientRect();return {x:r.x,y:r.y,width:r.width,height:r.height}})()`,
  );
  const viewport = await remote.evaluate(`({width:innerWidth,height:innerHeight})`);
  check(
    'child viewport fills workflow content, not sidebar/title',
    Math.abs(viewport.width - layout.width) < 4 &&
      Math.abs(viewport.height - layout.height) < 4 &&
      layout.x > 100 &&
      layout.y > 20,
  );
  await main.evaluate(`window.__TAURI_INTERNALS__.invoke('reload_workflows')`);
  await until(() =>
    remote
      .evaluate(`Boolean(document.querySelector('#login-form')) && !window.__desktopTestMarker`)
      .catch(() => false),
  );
  check(
    'explicit reconnect reloads the pinned service without replaying a task',
    await remote.evaluate(
      `location.href==='https://api.cyapi.cyou/workflows/' && document.querySelector('#api-key').value===''`,
    ),
  );
  await until(() => remote.evaluate(`document.documentElement.dataset.theme==='dark'`));
  const screenshot = await remote.call('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, 'remote-login.png'), Buffer.from(screenshot.data, 'base64'));
  await writeFile(
    path.join(output, 'report.json'),
    JSON.stringify(
      {
        executable,
        checks,
        layout,
        viewport,
        productionKeyUsed: false,
        paidCalls: false,
        fullBusinessFlowVerified: false,
      },
      null,
      2,
    ),
  );
  console.log(`Desktop check report: ${output}`);
} finally {
  for (const ws of connections) ws.close();
  if (child.pid && child.exitCode === null)
    spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
}
