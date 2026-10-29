// Real Windows EXE + WebView2 smoke check. Uses a fresh temporary user profile,
// no Agent credentials and no paid model. Verifies that the user workflow
// workbench is a native React page in the main WebView: there is no second
// page, no child WebView and no remote `/workflows/` UI.
// CDP is enabled only on this child process and removed when the EXE exits.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile, writeFile } from 'node:fs/promises';
import { createHash } from 'node:crypto';
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
    CYAPI_TOOLBOX_LOCAL_TEST_ROOT: output,
    CYAPI_TOOLBOX_HOME_DIR: path.join(output, 'home'),
    CYAPI_TOOLBOX_DATA_DIR: path.join(output, 'data'),
    WEBVIEW2_USER_DATA_FOLDER: path.join(output, 'webview'),
    WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
  },
  stdio: 'ignore',
});
const connections = [];
let main;
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
  main = await connect(local);
  await until(() => main.evaluate(`Boolean(document.querySelector('.n-workbenches'))`));
  check(
    'standard client exposes peer Tools / Workflows entries',
    await main.evaluate(
      `Array.from(document.querySelectorAll('.n-workbenches strong')).map(el=>el.textContent).join('|') === '工具接入|工作流'`,
    ),
  );

  // Switch to Workflows. The page is React in the main WebView, so the CDP
  // target list must not grow and the URL must stay the packaged app URL.
  await main.evaluate(
    `Array.from(document.querySelectorAll('.n-workbenches button')).find(el=>el.textContent.includes('工作流')).click();true`,
  );
  await until(() => main.evaluate(`Boolean(document.querySelector('.wfx'))`));
  check(
    'workflows open as a native page in the main WebView',
    await main.evaluate(`window.top === window && !document.querySelector('.wfx iframe')`),
  );
  const targets = await pages();
  check(
    'no remote workflow page or child WebView is created',
    targets.length === 1 &&
      targets.every(
        (page) =>
          page.url.startsWith('http://tauri.localhost') &&
          !/\/workflows\//.test(page.url) &&
          !/\/api\//.test(page.url),
      ),
  );
  check(
    'workflow heading is native and still names the section',
    await main.evaluate(
      `(()=>{const h=document.querySelector('.wfx-heading h2');return h?.textContent?.includes('工作流库')})()`,
    ),
  );

  // The template library is public, so a guest profile can browse it without a Key.
  await until(
    () => main.evaluate(`document.querySelectorAll('.wfx-card').length >= 2`),
    60000,
  );
  check('guest template library loads without activating a Key', true);
  check(
    'the client owns Key management instead of a server session',
    await main.evaluate(
      `document.querySelector('.wfx-key-chip')?.textContent.includes('激活 Key')`,
    ),
  );

  // Invalid Key is rejected in the same native dialog.
  await main.evaluate(`document.querySelector('.wfx-key-chip').click();true`);
  await until(() => main.evaluate(`Boolean(document.querySelector('.wfx-keys-dialog'))`));
  await main.evaluate(
    `(()=>{const form=document.querySelector('.wfx-key-add form');const set=Object.getOwnPropertyDescriptor(HTMLInputElement.prototype,'value').set;const name=form.querySelector('input:not([type=password]):not([type=checkbox])');const key=form.querySelector('input[type=password]');set.call(name,'桌面验收');name.dispatchEvent(new Event('input',{bubbles:true}));set.call(key,'invalid-desktop-smoke-key');key.dispatchEvent(new Event('input',{bubbles:true}));form.requestSubmit();return true})()`,
  );
  await until(
    () => main.evaluate(`Boolean(document.querySelector('.wfx-error')?.textContent)`),
    60000,
  );
  check(
    'invalid Key fails without creating a server session',
    await main.evaluate(
      `Boolean(document.querySelector('.wfx-error').textContent) && !localStorage.getItem('cyapi.workflows.keys.v1')`,
    ),
  );
  await main.evaluate(
    `document.dispatchEvent(new KeyboardEvent('keydown',{key:'Escape',bubbles:true}));true`,
  );
  await until(() => main.evaluate(`!document.querySelector('.wfx-keys-dialog')`));

  // Commands removed with the remote UI must not exist any more.
  check(
    'remote workflow layout command is gone',
    await main.evaluate(
      `(async()=>{try{await window.__TAURI_INTERNALS__.invoke('workflows',{layout:{visible:false}});return false}catch{return true}})()`,
    ),
  );
  check(
    'reload_workflows command is gone',
    await main.evaluate(
      `(async()=>{try{await window.__TAURI_INTERNALS__.invoke('reload_workflows');return false}catch{return true}})()`,
    ),
  );
  check(
    'local core permissions still work without a child view',
    typeof (await main.evaluate(`window.__TAURI_INTERNALS__.invoke('plugin:app|version')`)) ===
      'string',
  );
  const version = await main.evaluate(`window.__TAURI_INTERNALS__.invoke('plugin:app|version')`);
  if (process.env.WORKFLOW_DESKTOP_TEST_VERSION)
    check(
      'native version matches new local build',
      version === process.env.WORKFLOW_DESKTOP_TEST_VERSION,
    );

  // Navigation and appearance stay in the native page.
  await main.evaluate(`document.querySelector('[aria-label="设置"]').click();true`);
  await until(() => main.evaluate(`Boolean(document.querySelector('.n-settings'))`));
  await main.evaluate(`document.querySelector('.n-settings-theme:nth-child(3)').click();true`);
  await main.evaluate(
    `Array.from(document.querySelectorAll('.n-workbenches button')).find(el=>el.textContent.includes('工作流')).click();true`,
  );
  await until(() => main.evaluate(`document.documentElement.dataset.theme==='dark'`));
  check(
    'workflow page survives settings without reloading',
    await main.evaluate(`Boolean(document.querySelector('.wfx')) && !document.querySelector('.wfx iframe')`),
  );

  const screenshot = await main.call('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, 'workflow-library.png'), Buffer.from(screenshot.data, 'base64'));
  await writeFile(
    path.join(output, 'report.json'),
    JSON.stringify(
      {
        executable,
        sha256: createHash('sha256')
          .update(await readFile(executable))
          .digest('hex'),
        version,
        checks,
        pages: targets.map((page) => page.url),
        productionKeyUsed: false,
        personalTemplateCheck: false,
        paidCalls: false,
        fullBusinessFlowVerified: false,
      },
      null,
      2,
    ),
  );
  console.log(`Desktop check report: ${output}`);
} catch (error) {
  if (main) {
    console.error(
      'Workflow UI at failure:',
      await main
        .evaluate(`({url:location.href,text:document.body.innerText.slice(0,4000)})`)
        .catch(() => null),
    );
    const shot = await main.call('Page.captureScreenshot', { format: 'png' }).catch(() => null);
    if (shot) await writeFile(path.join(output, 'failure.png'), Buffer.from(shot.data, 'base64'));
  }
  console.error('Desktop diagnostics:', output);
  throw error;
} finally {
  for (const ws of connections) ws.close();
  if (child.pid && child.exitCode === null)
    spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
}
