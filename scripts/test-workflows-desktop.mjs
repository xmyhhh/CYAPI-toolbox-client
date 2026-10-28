// Real Windows EXE + WebView2 smoke check. Uses a fresh temporary user profile,
// no Agent credentials or paid model. Optional loopback-only personal-template
// checks use the dedicated smoke account and clean up their own templates/files.
// CDP is enabled only on this child process and removed when the EXE exits.
import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { mkdir, readFile, readdir, rm, writeFile } from 'node:fs/promises';
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
const origin = process.env.WORKFLOW_DESKTOP_TEST_ORIGIN || 'https://api.cyapi.cyou';
assert.ok(['https://api.cyapi.cyou', 'http://127.0.0.1:18732'].includes(origin));
const workflowUrl = origin + '/workflows/';
const personalCheck = process.env.WORKFLOW_DESKTOP_TEST_PERSONAL === '1';
const downloads = process.env.WORKFLOW_DESKTOP_TEST_DOWNLOADS;
if (personalCheck) {
  assert.equal(origin, 'http://127.0.0.1:18732');
  assert.ok(downloads && path.isAbsolute(downloads));
}
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
let main, remote;
const personalIds = new Set();
let exportedFile;
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
  // Saved navigation is absent in this isolated profile; default workflow must
  // load without starting a local engine or selecting/configuring an Agent.
  const target = await until(
    async () => (await pages()).find((page) => page.url === workflowUrl),
    60000,
  );
  remote = await connect(target);
  await until(
    () => remote.evaluate(`document.querySelectorAll('.workflow-card').length >= 2`),
    60000,
  );
  check(
    'guest template library loads in the native child view',
    await remote.evaluate(
      `window.top===window && location.origin===${JSON.stringify(origin)} && !document.querySelector('#app iframe') && !document.querySelector('#api-key')`,
    ),
  );
  await until(() => remote.evaluate(`document.documentElement.dataset.embedded === 'true'`));
  await until(() => main.evaluate(`!document.querySelector('.wf-offline')`));
  check(
    'workflow heading omits mode notice and browser/reconnect controls',
    await main.evaluate(
      `(()=>{const heading=document.querySelector('.n-page-heading');return heading?.querySelector('h1')?.textContent==='工作流' && !heading.querySelector('.n-heading-actions') && !/执行模式与费用|浏览器打开|重新连接/.test(heading.textContent)})()`,
    ),
  );
  await remote.evaluate(
    `document.querySelector('[data-action="example"][data-id="mock-video"]').click();true`,
  );
  await until(() => remote.evaluate(`Boolean(document.querySelector('.example-output video'))`));
  await remote.evaluate(
    `(()=>{const video=document.querySelector('.example-output video');video.muted=true;return video.play().then(()=>true)})()`,
  );
  await until(() =>
    remote.evaluate(`document.querySelector('.example-output video').currentTime>0.1`),
  );
  check('example video plays in native WebView', true);
  await remote.evaluate(`document.querySelector('#modal [data-action="close"]').click();true`);
  await remote.evaluate(
    `document.querySelector('[data-action="example"][data-id="mock-mesh"]').click();true`,
  );
  await until(() =>
    remote.evaluate(`document.querySelector('.example-output canvas')?.dataset.ready==='true'`),
  );
  check('example mesh renders in native WebView', true);
  await remote.evaluate(`document.querySelector('#modal [data-action="close"]').click();true`);
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
  const version = await main.evaluate(`window.__TAURI_INTERNALS__.invoke('plugin:app|version')`);
  if (process.env.WORKFLOW_DESKTOP_TEST_VERSION)
    check(
      'native version matches new local build',
      version === process.env.WORKFLOW_DESKTOP_TEST_VERSION,
    );
  check(
    'unauthenticated private endpoint remains 401',
    (await remote.evaluate(`fetch('/api/workflows/state').then(res=>res.status)`)) === 401,
  );
  await remote.evaluate(
    `document.querySelector('[data-action="create-draft"][data-id="image-idea"]').click();true`,
  );
  await until(() => remote.evaluate(`Boolean(document.querySelector('#description'))`));
  await remote.evaluate(`document.querySelector('[data-action="run"]').click();true`);
  await until(() =>
    remote.evaluate(`document.querySelector('#modal-title')?.textContent==='未激活 Key'`),
  );
  check(
    'missing Key shows a dialog without opening a form',
    await remote.evaluate(`!document.querySelector('#key-form')`),
  );
  await remote.evaluate(`document.querySelector('#modal [data-action="keys"]').click();true`);
  await until(() => remote.evaluate(`Boolean(document.querySelector('#key-form'))`));
  await until(() => main.evaluate(`!document.querySelector('.wf-offline')`));
  check(
    'Key management is ready in desktop host',
    await remote.evaluate(`location.hash==='#keys'`),
  );
  await remote.evaluate(
    `document.querySelector('#key-name').value='桌面验收';document.querySelector('#api-key').value='invalid-desktop-smoke-key';document.querySelector('#key-form').requestSubmit();true`,
  );
  await until(
    () => remote.evaluate(`Boolean(document.querySelector('#key-error')?.textContent)`),
    45000,
  );
  check(
    'invalid Key is rejected and cleared from input',
    await remote.evaluate(
      `document.querySelector('#api-key').value==='' && Boolean(document.querySelector('#key-error').textContent)`,
    ),
  );
  if (personalCheck) {
    const keyResult = spawnSync(
      'docker',
      [
        'exec',
        '-i',
        'cyapi-workflow-local-postgres-1',
        'psql',
        '-U',
        'sub2api_test',
        '-d',
        'sub2api_test',
        '-A',
        '-t',
      ],
      { input: 'SELECT key FROM api_keys WHERE id=3 AND user_id=4', encoding: 'utf8' },
    );
    assert.equal(keyResult.status, 0, 'local smoke fixture is available');
    const key = keyResult.stdout.trim();
    assert.ok(key.length > 10);
    const name = '桌面导出验收-' + crypto.randomUUID();
    await remote.evaluate(
      `document.querySelector('#key-name').value='桌面验收';document.querySelector('#remember-key').checked=false;document.querySelector('#api-key').value=${JSON.stringify(key)};document.querySelector('#key-form').requestSubmit();true`,
    );
    await until(() =>
      remote.evaluate(
        `document.querySelector('.active-key-status')?.textContent.includes('桌面验收') && !document.querySelector('#api-key').value`,
      ),
    );
    const created = await remote.evaluate(
      `(async()=>{const s=await (await fetch('/api/workflows/session')).json();const r=await fetch('/api/workflows/templates',{method:'POST',headers:{'Content-Type':'application/json','X-CSRF-Token':s.csrf},body:JSON.stringify({fromId:'mock-mesh',name:${JSON.stringify(name)}})});if(!r.ok)throw Error('template setup failed');return (await r.json()).id})()`,
    );
    personalIds.add(created);
    await remote.evaluate(
      `Array.from(document.querySelectorAll('[data-action="library"]')).at(0).click();true`,
    );
    await until(() =>
      remote.evaluate(`Boolean(document.querySelector('[data-template-id="${created}"]'))`),
    );
    const existing = new Set(await readdir(downloads).catch(() => []));
    await remote.evaluate(
      `document.querySelector('[data-template-id="${created}"] [data-action="template-export"]').click();true`,
    );
    exportedFile = await until(async () => {
      for (const file of await readdir(downloads).catch(() => [])) {
        if (existing.has(file) || !/^workflow-[0-9a-f-]+\.json$/.test(file)) continue;
        const candidate = path.join(downloads, file);
        const value = await readFile(candidate, 'utf8')
          .then(JSON.parse)
          .catch(() => null);
        if (value?.name === name && value.format === 'cyapi-workflow-v1') return candidate;
      }
    });
    check(
      'personal workflow exports a usable JSON through native downloads',
      Boolean(exportedFile),
    );
    await remote.evaluate(
      `document.querySelector('[data-template-id="${created}"] [data-action="template-delete"]').click();true`,
    );
    await until(() =>
      remote.evaluate(`Boolean(document.querySelector('[data-action="template-delete-confirm"]'))`),
    );
    await remote.evaluate(
      `document.querySelector('[data-action="template-delete-confirm"]').click();true`,
    );
    await until(() =>
      remote.evaluate(`!document.querySelector('[data-template-id="${created}"]')`),
    );
    personalIds.delete(created);
    check('personal workflow deletion removes the card in native WebView', true);
    await remote.evaluate(`document.querySelector('[data-action="keys"]').click();true`);
    await until(() =>
      remote.evaluate(
        `Boolean(document.querySelector('.active-key-status [data-action="logout"]'))`,
      ),
    );
    await remote.evaluate(
      `document.querySelector('.active-key-status [data-action="logout"]').click();true`,
    );
    await until(() =>
      remote.evaluate(`document.querySelector('.active-key-status')?.textContent.includes('无')`),
    );
  }
  await main.evaluate(
    `Array.from(document.querySelectorAll('.primary-nav button')).find(el=>el.textContent.includes('任务队列')).click();true`,
  );
  await until(() =>
    remote.evaluate(`document.querySelector('.tabs .active')?.dataset.action==='queue'`),
  );
  check(
    'desktop navigation leaves Key management',
    await main.evaluate(`!document.querySelector('.wf-offline')`),
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
      .evaluate(`Boolean(document.querySelector('.workflow-card')) && !window.__desktopTestMarker`)
      .catch(() => false),
  );
  check(
    'explicit reconnect reloads the pinned service without replaying a task',
    await remote.evaluate(
      `location.href===${JSON.stringify(workflowUrl)} && !document.querySelector('#api-key')`,
    ),
  );
  await until(() => remote.evaluate(`document.documentElement.dataset.theme==='dark'`));
  const screenshot = await remote.call('Page.captureScreenshot', { format: 'png' });
  await writeFile(
    path.join(output, 'workflow-library.png'),
    Buffer.from(screenshot.data, 'base64'),
  );
  const mainScreenshot = await main.call('Page.captureScreenshot', { format: 'png' });
  await writeFile(path.join(output, 'desktop.png'), Buffer.from(mainScreenshot.data, 'base64'));
  await writeFile(
    path.join(output, 'report.json'),
    JSON.stringify(
      {
        executable,
        sha256: createHash('sha256')
          .update(await readFile(executable))
          .digest('hex'),
        version,
        workflowUrl,
        checks,
        layout,
        viewport,
        productionKeyUsed: false,
        personalTemplateCheck: personalCheck,
        paidCalls: false,
        fullBusinessFlowVerified: false,
      },
      null,
      2,
    ),
  );
  console.log(`Desktop check report: ${output}`);
} catch (error) {
  if (remote) {
    console.error(
      'Workflow UI at failure:',
      await remote
        .evaluate(
          `({url:location.href,text:document.body.innerText,disabled:Array.from(document.querySelectorAll('[data-action][disabled]')).map(e=>e.dataset.action)})`,
        )
        .catch(() => null),
    );
    const shot = await remote.call('Page.captureScreenshot', { format: 'png' }).catch(() => null);
    if (shot) await writeFile(path.join(output, 'failure.png'), Buffer.from(shot.data, 'base64'));
  }
  console.error('Desktop diagnostics:', output);
  throw error;
} finally {
  if (remote && personalIds.size) {
    await remote
      .evaluate(
        `(async()=>{const s=await(await fetch('/api/workflows/session')).json();for(const id of ${JSON.stringify([...personalIds])})await fetch('/api/workflows/templates/'+id,{method:'DELETE',headers:{'X-CSRF-Token':s.csrf}})})()`,
      )
      .catch(() => {});
  }
  if (exportedFile) await rm(exportedFile, { force: true });
  for (const ws of connections) ws.close();
  if (child.pid && child.exitCode === null)
    spawnSync('taskkill', ['/PID', String(child.pid), '/T', '/F'], { stdio: 'ignore' });
}
